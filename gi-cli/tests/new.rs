//! Black-box integration tests for `gi new`.
//!
//! These drive the compiled `gi` binary against a throwaway temp repo backed
//! by a *real* `git` — the tool's contract is "shell out to the user's git",
//! so faithful tests use real git rather than a fake. Assertions are on
//! observable outcomes only: the files under `.issues/` and the resulting
//! `git log`. The editor is a scripted stand-in pointed at via `$EDITOR`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use tempfile::TempDir;

/// A line the fake editor appends, standing in for the user writing a body.
const EDITED_MARKER: &str = "EDITED_BODY_MARKER";

/// Run `git` in `repo` with a hermetic config (no user global/system config
/// leaking in), panicking on failure.
fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .status()
        .expect("failed to run git");
    assert!(status.success(), "git {args:?} failed");
}

/// Capture stdout of a `git` command in `repo`, trimmed.
fn git_out(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("failed to run git");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Initialize a temp repo with a hermetic identity and signing disabled.
fn init_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "tester@example.com"]);
    git(dir.path(), &["config", "user.name", "Tester"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

/// Write an executable shell script that simulates the user editing: it
/// appends a marker line to the file it is handed.
fn write_fake_editor(dir: &Path) -> PathBuf {
    let path = dir.join("fake-editor.sh");
    fs::write(
        &path,
        format!("#!/bin/sh\nprintf '\\n%s\\n' '{EDITED_MARKER}' >> \"$1\"\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// Build a `gi` command rooted in `repo` with the fake editor wired in and a
/// hermetic git environment.
fn gi(repo: &Path, editor: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gi").unwrap();
    cmd.current_dir(repo)
        .env("EDITOR", editor)
        .env("VISUAL", editor)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    cmd
}

/// The single `.md` file under `.issues/`, asserting there is exactly one.
fn sole_issue_file(repo: &Path) -> PathBuf {
    let entries: Vec<PathBuf> = fs::read_dir(repo.join(".issues"))
        .expect(".issues should exist")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    assert_eq!(entries.len(), 1, "expected exactly one issue file");
    entries.into_iter().next().unwrap()
}

#[test]
fn new_from_empty_repo_creates_dir_file_and_commit() {
    let repo = init_repo();
    let editor = write_fake_editor(repo.path());

    gi(repo.path(), &editor)
        .args(["new", "Fix login bug"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created issue"));

    // .issues/ was auto-created with exactly one issue file, named by slug.
    let file = sole_issue_file(repo.path());
    let name = file.file_name().unwrap().to_string_lossy();
    assert!(
        name.starts_with("fix-login-bug-") && name.ends_with(".md"),
        "unexpected issue filename: {name}"
    );

    // Frontmatter is well-formed and the editor's body edit was preserved.
    let contents = fs::read_to_string(&file).unwrap();
    assert!(contents.starts_with("---\n"));
    assert!(contents.contains("\ntitle: Fix login bug\n"));
    assert!(contents.contains("\nstatus: open\n"));
    assert!(contents.contains("\nassignee:"));
    assert!(contents.contains(EDITED_MARKER), "editor edit was not saved");

    // Exactly one commit, message names the id, tree holds only the issue file.
    assert_eq!(git_out(repo.path(), &["rev-list", "--count", "HEAD"]), "1");
    let subject = git_out(repo.path(), &["log", "-1", "--pretty=%s"]);
    assert!(subject.starts_with("issue: new "), "subject: {subject}");
    let committed = git_out(
        repo.path(),
        &["diff-tree", "--root", "--no-commit-id", "--name-only", "-r", "HEAD"],
    );
    assert_eq!(committed, format!(".issues/{name}"));
}

#[test]
fn new_does_not_sweep_in_unrelated_staged_work() {
    let repo = init_repo();
    let editor = write_fake_editor(repo.path());

    // An initial commit so HEAD exists, then unrelated staged work.
    fs::write(repo.path().join("README.md"), "hello\n").unwrap();
    git(repo.path(), &["add", "README.md"]);
    git(repo.path(), &["commit", "-m", "init"]);

    fs::write(repo.path().join("unrelated.txt"), "do not commit me\n").unwrap();
    git(repo.path(), &["add", "unrelated.txt"]);

    gi(repo.path(), &editor)
        .args(["new", "Another issue"])
        .assert()
        .success();

    // The issue commit holds ONLY the issue file — unrelated.txt is excluded.
    let name = sole_issue_file(repo.path())
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let committed = git_out(
        repo.path(),
        &["diff-tree", "--root", "--no-commit-id", "--name-only", "-r", "HEAD"],
    );
    assert_eq!(committed, format!(".issues/{name}"));

    // unrelated.txt is still staged but uncommitted (never reached any commit).
    let staged = git_out(repo.path(), &["diff", "--cached", "--name-only"]);
    assert_eq!(staged, "unrelated.txt");
}

#[test]
fn new_requires_a_title() {
    let repo = init_repo();
    let editor = write_fake_editor(repo.path());

    // clap rejects a missing positional before our code runs.
    gi(repo.path(), &editor).arg("new").assert().failure();
}

#[test]
fn new_outside_a_git_repo_fails_cleanly() {
    // A temp dir that is deliberately NOT a git repo.
    let dir = TempDir::new().unwrap();
    let editor = write_fake_editor(dir.path());

    gi(dir.path(), &editor)
        .args(["new", "Nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a git repository"));
}
