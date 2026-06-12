//! Black-box tests for `gi board`.
//!
//! The interactive TUI itself can't be driven without a live terminal — its
//! rendering is covered by `TestBackend` snapshots in `src/board.rs`. What we
//! *can* assert here is the boundary before the terminal is ever touched: the
//! board loads through the same validating read path as `gi list`, so a
//! malformed issue is rejected up front rather than dropping the user into a
//! board built from partial data.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use tempfile::TempDir;

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

fn init_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "tester@example.com"]);
    git(dir.path(), &["config", "user.name", "Tester"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

fn seed_raw(repo: &Path, filename: &str, contents: &str) {
    let issues = repo.join(".issues");
    fs::create_dir_all(&issues).unwrap();
    fs::write(issues.join(filename), contents).unwrap();
}

fn gi(repo: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gi").unwrap();
    cmd.current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    cmd
}

#[test]
fn board_rejects_a_malformed_issue_before_launching() {
    let repo = init_repo();
    seed_raw(
        repo.path(),
        "bogus-eeee.md",
        "---\nid: eeee\ntitle: Bogus\nstatus: wat\nassignee:\n---\nbody\n",
    );

    // The read path fails before any terminal setup, so this returns rather than
    // hanging on a TUI event loop.
    gi(repo.path())
        .arg("board")
        .assert()
        .failure()
        .stderr(predicate::str::contains("bogus-eeee.md"))
        .stderr(predicate::str::contains("unknown status"));
}

#[test]
fn board_outside_a_git_repo_fails_cleanly() {
    let dir = TempDir::new().unwrap();
    gi(dir.path())
        .arg("board")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a git repository"));
}
