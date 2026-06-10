//! Thin traits over the two side effects `gi` performs: opening the user's
//! editor and committing through the user's `git` binary. Keeping them behind
//! traits lets command orchestration be driven with stand-ins, while the
//! default implementations defer entirely to the user's environment.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Opens a file for the user to edit interactively.
pub trait Editor {
    fn edit(&self, path: &Path) -> Result<()>;
}

/// Commits changes through git, scoped to specific paths.
pub trait Git {
    /// Discover the repository root (working tree top level) from `cwd`.
    fn repo_root(&self, cwd: &Path) -> Result<std::path::PathBuf>;

    /// Stage and commit exactly `rel_paths` (relative to `repo_root`) with
    /// `message`. Other staged or in-flight work must not be swept in.
    fn commit(&self, repo_root: &Path, rel_paths: &[&str], message: &str) -> Result<()>;
}

/// Launches `$VISUAL`/`$EDITOR` (falling back to `vi`) on the given path,
/// inheriting the user's terminal so interactive editors work.
pub struct SystemEditor;

impl Editor for SystemEditor {
    fn edit(&self, path: &Path) -> Result<()> {
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".to_string());

        // Route through `sh -c` so an `$EDITOR` carrying its own flags (e.g.
        // "code -w") is honored. `$1` is the file; `sh` fills `$0`.
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("sh")
            .arg(path)
            .status()
            .with_context(|| format!("failed to launch editor `{editor}`"))?;

        if !status.success() {
            bail!("editor `{editor}` exited with a non-zero status");
        }
        Ok(())
    }
}

/// Shells out to the user's `git` binary, inheriting their config,
/// credentials, hooks and commit signing.
pub struct SystemGit;

impl SystemGit {
    fn run(&self, repo_root: Option<&Path>, args: &[&std::ffi::OsStr]) -> Result<()> {
        let mut cmd = Command::new("git");
        if let Some(root) = repo_root {
            cmd.arg("-C").arg(root);
        }
        cmd.args(args);
        let output = cmd.output().context("failed to run git")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git command failed: {}", stderr.trim());
        }
        Ok(())
    }
}

impl Git for SystemGit {
    fn repo_root(&self, cwd: &Path) -> Result<std::path::PathBuf> {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("failed to run git")?;
        if !output.status.success() {
            bail!("not inside a git repository (gi stores issues alongside your code)");
        }
        let root = String::from_utf8(output.stdout)
            .context("git returned non-utf8 path")?
            .trim()
            .to_string();
        Ok(std::path::PathBuf::from(root))
    }

    fn commit(&self, repo_root: &Path, rel_paths: &[&str], message: &str) -> Result<()> {
        use std::ffi::OsStr;

        // Stage only the named paths.
        let mut add: Vec<&OsStr> = vec![OsStr::new("add"), OsStr::new("--")];
        add.extend(rel_paths.iter().map(OsStr::new));
        self.run(Some(repo_root), &add)?;

        // Commit with an explicit pathspec so a partial commit is taken: any
        // other staged content is left untouched in the index.
        let mut commit: Vec<&OsStr> = vec![
            OsStr::new("commit"),
            OsStr::new("-m"),
            OsStr::new(message),
            OsStr::new("--"),
        ];
        commit.extend(rel_paths.iter().map(OsStr::new));
        self.run(Some(repo_root), &commit)?;
        Ok(())
    }
}
