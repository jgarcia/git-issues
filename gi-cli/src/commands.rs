//! Command orchestration. Each command wires `gi-core`'s pure logic to the
//! `Editor`/`Git` effects, so the flow can be exercised with stand-ins.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use gi_core::{new_hash, Issue, Status};

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
    git.commit(
        &repo_root,
        &[&rel_path],
        &format!("issue: new {}", issue.id),
    )?;

    Ok(Created {
        id: issue.id,
        rel_path,
    })
}

/// Which slice of the backlog `gi list` renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// The default: not-done work (open + in progress).
    NotDone,
    /// Every issue, done included.
    All,
    /// Only done issues.
    Done,
}

impl Filter {
    /// Whether an issue in `status` should appear under this filter.
    fn admits(self, status: Status) -> bool {
        match self {
            Filter::NotDone => status != Status::Done,
            Filter::All => true,
            Filter::Done => status == Status::Done,
        }
    }
}

/// Read and validate every issue under `.issues/`, then render the matching
/// ones as a `hash  status  who  title` table.
///
/// This is the read path: each file is parsed through `gi-core`'s validating
/// reader. If *any* file is malformed — unknown status, missing field, leftover
/// git conflict markers — the command fails and reports every offending file,
/// rather than silently dropping it from the listing.
pub fn list(git: &dyn Git, cwd: &Path, filter: Filter) -> Result<String> {
    let repo_root = git.repo_root(cwd)?;
    let issues_dir = repo_root.join(".issues");

    // No `.issues/` yet (e.g. before the first `gi new`) is an empty backlog,
    // not an error.
    let read_dir = match fs::read_dir(&issues_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty_listing()),
        Err(e) => {
            return Err(e).with_context(|| format!("failed to read {}", issues_dir.display()));
        }
    };

    // Collect `.md` filenames up front and sort them so both the error report
    // and (as a tiebreak) the rendered table are deterministic.
    let mut names: Vec<String> = Vec::new();
    for entry in read_dir {
        let entry = entry.with_context(|| format!("failed to read {}", issues_dir.display()))?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "md") {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();

    let mut issues: Vec<Issue> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for name in &names {
        let contents = fs::read_to_string(issues_dir.join(name))
            .with_context(|| format!("failed to read .issues/{name}"))?;
        match Issue::parse(&contents) {
            Ok(issue) => issues.push(issue),
            Err(e) => errors.push(format!("  .issues/{name}: {e}")),
        }
    }

    // Validation-on-read: surface every bad file and refuse to render a
    // possibly-misleading partial view.
    if !errors.is_empty() {
        let noun = if errors.len() == 1 { "issue" } else { "issues" };
        bail!(
            "{} invalid {noun} in .issues/:\n{}",
            errors.len(),
            errors.join("\n")
        );
    }

    let mut rows: Vec<&Issue> = issues.iter().filter(|i| filter.admits(i.status)).collect();
    // Workflow order, then title, then id for a stable, intuitive listing.
    rows.sort_by(|a, b| {
        a.status
            .rank()
            .cmp(&b.status.rank())
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });

    if rows.is_empty() {
        return Ok(empty_listing());
    }
    Ok(render_table(&rows))
}

/// The message shown when no issues match the current filter.
fn empty_listing() -> String {
    "No issues.".to_string()
}

/// Render issues as a left-aligned `hash  status  who  title` table with a
/// header row. Columns are padded to the widest cell and separated by two
/// spaces; an unassigned issue shows `-` in the `who` column.
fn render_table(issues: &[&Issue]) -> String {
    const HEADERS: [&str; 4] = ["hash", "status", "who", "title"];

    let cells: Vec<[String; 4]> = issues
        .iter()
        .map(|i| {
            [
                i.id.clone(),
                i.status.as_str().to_string(),
                i.assignee.clone().unwrap_or_else(|| "-".to_string()),
                i.title.clone(),
            ]
        })
        .collect();

    // Widths span the header and every row; the last column needs no padding.
    let mut widths = HEADERS.map(|h| h.len());
    for row in &cells {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }

    let mut out = String::new();
    let render_row = |out: &mut String, row: &[String; 4]| {
        for col in 0..4 {
            if col > 0 {
                out.push_str("  ");
            }
            out.push_str(&row[col]);
            // Pad every column but the last to keep the title flush-left.
            if col < 3 {
                for _ in 0..widths[col] - row[col].chars().count() {
                    out.push(' ');
                }
            }
        }
        out.push('\n');
    };

    render_row(&mut out, &HEADERS.map(|h| h.to_string()));
    for row in &cells {
        render_row(&mut out, row);
    }
    // Trim the trailing newline; the caller's `println!` re-adds one.
    out.truncate(out.trim_end_matches('\n').len());
    out
}
