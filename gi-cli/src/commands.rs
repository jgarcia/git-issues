//! Command orchestration. Each command wires `gi-core`'s pure logic to the
//! `Editor`/`Git` effects, so the flow can be exercised with stand-ins.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use gi_core::{new_hash, Issue};

use crate::effects::{Editor, Git};

/// What `new` produced, for reporting back to the user.
pub struct Created {
    pub id: String,
    /// Path to the issue file, relative to the repo root.
    pub rel_path: String,
}

/// Create an issue end-to-end: write the file (auto-creating `.issues/` on
/// first use), open it in the editor for the body, then commit it on its own.
pub fn new(editor: &dyn Editor, git: &dyn Git, cwd: &Path, title: &str) -> Result<Created> {
    let title = title.trim();
    if title.is_empty() {
        bail!("an issue title is required: `gi new <title>`");
    }

    let repo_root = git.repo_root(cwd)?;
    let issues_dir = repo_root.join(".issues");
    fs::create_dir_all(&issues_dir)
        .with_context(|| format!("failed to create {}", issues_dir.display()))?;

    // Pick an id whose file does not already exist. Collisions are rare; a few
    // attempts is plenty before we surface the (effectively impossible) failure.
    let mut issue = Issue::create(String::new(), title);
    let mut filename = String::new();
    let mut found = false;
    for _ in 0..16 {
        issue.id = new_hash();
        filename = issue.filename();
        if !issues_dir.join(&filename).exists() {
            found = true;
            break;
        }
    }
    if !found {
        bail!("could not allocate a free issue id; try again");
    }

    let abs_path = issues_dir.join(&filename);
    fs::write(&abs_path, issue.to_file_string())
        .with_context(|| format!("failed to write {}", abs_path.display()))?;

    editor.edit(&abs_path)?;

    let rel_path = format!(".issues/{filename}");
    git.commit(&repo_root, &[&rel_path], &format!("issue: new {}", issue.id))?;

    Ok(Created {
        id: issue.id,
        rel_path,
    })
}
