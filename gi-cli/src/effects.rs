//! Thin traits over the two side effects `gi` performs: opening the user's
//! editor and committing through the user's `git` binary. Keeping them behind
//! traits lets command orchestration be driven with stand-ins, while the
//! default implementations defer entirely to the user's environment.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use gi_core::{parse_shortlog, Committer};

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

    /// The set of people who have authored commits reachable from `HEAD` — the
    /// valid pool of assignees. Implementations should cache the derived set
    /// rather than re-running `git shortlog` on every command.
    fn committers(&self, repo_root: &Path) -> Result<Vec<Committer>>;

    /// The current user's identity from `git config user.name` / `user.email`.
    /// Used by `gi grab` to self-assign without a committer-validity check.
    fn current_user(&self, repo_root: &Path) -> Result<Committer>;
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

    /// Read a single git config value from the repo, returning `None` if unset.
    fn config_get(&self, repo_root: &Path, key: &str) -> Result<Option<String>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["config", "--get", key])
            .output()
            .context("failed to run git")?;
        if !output.status.success() {
            return Ok(None);
        }
        let val = String::from_utf8(output.stdout).context("git returned non-utf8 output")?;
        let val = val.trim().to_string();
        Ok(if val.is_empty() { None } else { Some(val) })
    }

    /// Run a read-only git command in `repo_root` and return its trimmed stdout,
    /// or `None` if git exits non-zero (e.g. `rev-parse HEAD` before any commit).
    fn capture(&self, repo_root: &Path, args: &[&str]) -> Result<Option<String>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(args)
            .output()
            .context("failed to run git")?;
        if !output.status.success() {
            return Ok(None);
        }
        let out = String::from_utf8(output.stdout).context("git returned non-utf8 output")?;
        Ok(Some(out))
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

    /// Derive the committer set from `git shortlog -sne HEAD`, cached in
    /// `.git/gi-committers` and keyed on the current `HEAD` sha. The set only
    /// changes when new commits land, so as long as `HEAD` is unmoved we reuse
    /// the cached shortlog instead of re-deriving it on every command. The cache
    /// lives under `.git/`, so it is local and never tracked.
    fn committers(&self, repo_root: &Path) -> Result<Vec<Committer>> {
        // No commits yet means no authors — and nothing to key a cache on.
        let head = match self.capture(repo_root, &["rev-parse", "HEAD"])? {
            Some(h) => h.trim().to_string(),
            None => return Ok(Vec::new()),
        };

        let cache_path = repo_root.join(".git").join("gi-committers");

        // A cache whose first line matches the current HEAD is still valid; the
        // remainder is the raw shortlog, re-parsed through the same core parser.
        if let Ok(cached) = fs::read_to_string(&cache_path) {
            if let Some((stored_head, shortlog)) = cached.split_once('\n') {
                if stored_head == head {
                    return Ok(parse_shortlog(shortlog));
                }
            }
        }

        let shortlog = self
            .capture(repo_root, &["shortlog", "-sne", "HEAD"])?
            .unwrap_or_default();

        // Best-effort write: a failed cache update must not fail the command, it
        // only costs us a re-derivation next time.
        let _ = fs::write(&cache_path, format!("{head}\n{shortlog}"));

        Ok(parse_shortlog(&shortlog))
    }

    fn current_user(&self, repo_root: &Path) -> Result<Committer> {
        let name = self.config_get(repo_root, "user.name")?.unwrap_or_default();
        let email = self.config_get(repo_root, "user.email")?.unwrap_or_default();
        if name.is_empty() && email.is_empty() {
            anyhow::bail!(
                "git user identity is not configured; \
                 set `user.name` or `user.email` with `git config`"
            );
        }
        Ok(Committer { name, email })
    }
}
