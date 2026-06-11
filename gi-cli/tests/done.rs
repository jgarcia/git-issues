//! Black-box integration tests for `gi done` — the close path.
//!
//! These drive the compiled `gi` binary against a throwaway temp repo backed
//! by a *real* `git`, mirroring the `new`/`list` suites. An issue is seeded and
//! committed first (so it is tracked, as it would be after `gi new`), then
//! `gi done` is run and assertions are made on observable outcomes only: the
//! file's `status` field and the resulting `git log` / committed tree.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use tempfile::TempDir;

/// Run `git` in `repo` with a hermetic config, panicking on failure.
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

/// Build a `gi` command rooted in `repo` with a hermetic git environment.
fn gi(repo: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gi").unwrap();
    cmd.current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    cmd
}

/// Seed an issue file under `.issues/` and commit it (so it is tracked, just as
/// `gi new` would leave it). Returns the file's name and relative path.
fn seed_committed_issue(repo: &Path, hash: &str, title: &str, status: &str) -> (String, String) {
    let issues = repo.join(".issues");
    fs::create_dir_all(&issues).unwrap();
    let slug = title.to_lowercase().replace(' ', "-");
    let name = format!("{slug}-{hash}.md");
    let rel = format!(".issues/{name}");
    let contents =
        format!("---\nid: {hash}\ntitle: {title}\nstatus: {status}\nassignee:\n---\nbody of {hash}\n");
    fs::write(issues.join(&name), contents).unwrap();
    git(repo, &["add", &rel]);
    git(repo, &["commit", "-m", "seed issue"]);
    (name, rel)
}

#[test]
fn done_sets_status_and_commits_only_that_file() {
    let repo = init_repo();
    let (_name, rel) = seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    // Unrelated staged work must not be swept into the close commit.
    fs::write(repo.path().join("unrelated.txt"), "do not commit me\n").unwrap();
    git(repo.path(), &["add", "unrelated.txt"]);

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path())
        .args(["done", "a1b2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Closed issue a1b2"));

    // The file's status flipped to done; the rest of the frontmatter/body is kept.
    let contents = fs::read_to_string(repo.path().join(&rel)).unwrap();
    assert!(contents.contains("\nstatus: done\n"), "status not done:\n{contents}");
    assert!(contents.contains("\ntitle: Fix login bug\n"));
    assert!(contents.contains("body of a1b2"));

    // Exactly one new commit, whose subject names the id.
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after.parse::<u32>().unwrap(), before.parse::<u32>().unwrap() + 1);
    let subject = git_out(repo.path(), &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject, "issue: close a1b2");

    // That commit holds ONLY the issue file — unrelated.txt is left untouched.
    let committed = git_out(
        repo.path(),
        &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
    );
    assert_eq!(committed, rel);
    let staged = git_out(repo.path(), &["diff", "--cached", "--name-only"]);
    assert_eq!(staged, "unrelated.txt");
}

#[test]
fn unknown_id_fails_and_makes_no_commit() {
    let repo = init_repo();
    seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path())
        .args(["done", "zzzz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue with id `zzzz`"));

    // No new commit, and the real issue is left open.
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after, before);
    let contents = fs::read_to_string(repo.path().join(".issues/fix-login-bug-a1b2.md")).unwrap();
    assert!(contents.contains("\nstatus: open\n"));
}

#[test]
fn already_done_is_a_no_op_without_a_commit() {
    let repo = init_repo();
    seed_committed_issue(repo.path(), "c4d5", "Ship release", "done");

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path())
        .args(["done", "c4d5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already done"));

    // Idempotent: no extra commit is created.
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after, before);
}

#[test]
fn done_outside_a_git_repo_fails_cleanly() {
    let dir = TempDir::new().unwrap();
    gi(dir.path())
        .args(["done", "a1b2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a git repository"));
}
