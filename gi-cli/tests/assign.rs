//! Black-box integration tests for `gi assign` — the "this is yours" verb.
//!
//! Like the `done` suite, these drive the compiled `gi` binary against a
//! throwaway temp repo backed by a *real* `git`. Assignees are resolved against
//! the repo's actual commit authors, so the seeded commit's identity (`Tester
//! <tester@example.com>`) is exactly the pool a successful assign matches into.
//! Assertions are on observable outcomes only: the issue file's fields and the
//! resulting `git log` / committed tree.

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

/// Initialize a temp repo with a hermetic identity and signing disabled. The
/// configured `user.name`/`user.email` become the repo's sole committer once an
/// issue is seeded, and thus the only resolvable assignee.
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
fn assign_sets_assignee_leaves_status_and_commits_only_that_file() {
    let repo = init_repo();
    let (_name, rel) = seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    // Unrelated staged work must not be swept into the assign commit.
    fs::write(repo.path().join("unrelated.txt"), "do not commit me\n").unwrap();
    git(repo.path(), &["add", "unrelated.txt"]);

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    // `tester` resolves fuzzily to the committer `Tester <tester@example.com>`.
    gi(repo.path())
        .args(["assign", "a1b2", "tester"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assigned issue a1b2 to Tester"));

    // The assignee was set; status is untouched (stays open); body is kept.
    let contents = fs::read_to_string(repo.path().join(&rel)).unwrap();
    assert!(
        contents.contains("\nassignee: Tester\n"),
        "assignee not set:\n{contents}"
    );
    assert!(
        contents.contains("\nstatus: open\n"),
        "status should stay open:\n{contents}"
    );
    assert!(contents.contains("body of a1b2"));

    // Exactly one new commit, whose subject names the id and assignee.
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after.parse::<u32>().unwrap(), before.parse::<u32>().unwrap() + 1);
    let subject = git_out(repo.path(), &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject, "issue: assign a1b2 to Tester");

    // That commit holds ONLY the issue file — unrelated.txt is left untouched.
    let committed = git_out(
        repo.path(),
        &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
    );
    assert_eq!(committed, rel);
    let staged = git_out(repo.path(), &["diff", "--cached", "--name-only"]);
    assert_eq!(staged, "unrelated.txt");

    // The committer set was cached under .git/, keyed on the (pre-assign) HEAD.
    let cache = fs::read_to_string(repo.path().join(".git/gi-committers")).unwrap();
    assert!(
        cache.contains("Tester <tester@example.com>"),
        "cache missing committer:\n{cache}"
    );
}

#[test]
fn assign_accepts_an_email_form() {
    let repo = init_repo();
    seed_committed_issue(repo.path(), "c3d4", "Add logout", "open");

    gi(repo.path())
        .args(["assign", "c3d4", "tester@example.com"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assigned issue c3d4 to Tester"));
}

#[test]
fn unresolvable_assignee_is_rejected_and_makes_no_commit() {
    let repo = init_repo();
    let (_name, rel) = seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path())
        .args(["assign", "a1b2", "nobody"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("matches no committer"));

    // No new commit, and the issue is left unassigned.
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after, before);
    let contents = fs::read_to_string(repo.path().join(&rel)).unwrap();
    assert!(
        contents.contains("\nassignee:\n"),
        "assignee should stay empty:\n{contents}"
    );
}

#[test]
fn unknown_id_fails_before_touching_anything() {
    let repo = init_repo();
    seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path())
        .args(["assign", "zzzz", "tester"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue with id `zzzz`"));

    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after, before);
}

#[test]
fn assign_outside_a_git_repo_fails_cleanly() {
    let dir = TempDir::new().unwrap();
    gi(dir.path())
        .args(["assign", "a1b2", "tester"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a git repository"));
}
