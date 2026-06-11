//! Black-box integration tests for `gi grab` — the self-assign & in-progress verb.
//!
//! Like the other verb suites, these drive the compiled `gi` binary against a
//! throwaway temp repo backed by a *real* `git`. The repo is configured with a
//! known identity (`Tester <tester@example.com>`), which becomes the expected
//! assignee value after a grab. Assertions are on observable outcomes only: the
//! issue file's fields and the resulting `git log` / committed tree.

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
fn grab_sets_assignee_and_in_progress_and_commits_only_that_file() {
    let repo = init_repo();
    let (_name, rel) = seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    // Unrelated staged work must not be swept into the grab commit.
    fs::write(repo.path().join("unrelated.txt"), "do not commit me\n").unwrap();
    git(repo.path(), &["add", "unrelated.txt"]);

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path())
        .args(["grab", "a1b2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Grabbed issue a1b2 for Tester"));

    // The assignee was set to the current user's name; status is in_progress.
    let contents = fs::read_to_string(repo.path().join(&rel)).unwrap();
    assert!(
        contents.contains("\nassignee: Tester\n"),
        "assignee not set:\n{contents}"
    );
    assert!(
        contents.contains("\nstatus: in_progress\n"),
        "status should be in_progress:\n{contents}"
    );
    assert!(contents.contains("body of a1b2"), "body should be preserved");

    // Exactly one new commit, whose subject names the id.
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after.parse::<u32>().unwrap(), before.parse::<u32>().unwrap() + 1);
    let subject = git_out(repo.path(), &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject, "issue: grab a1b2");

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
fn grab_succeeds_even_when_user_is_not_yet_a_committer() {
    // A fresh repo has no commits, so there are no committers in the shortlog.
    // `gi grab` must succeed anyway — it never validates against the committer set.
    let repo = init_repo();

    // Seed the issue without a prior seed commit so the user has no commit history.
    let issues = repo.path().join(".issues");
    fs::create_dir_all(&issues).unwrap();
    let name = "fix-login-bug-c3d4.md";
    let rel = ".issues/fix-login-bug-c3d4.md";
    let contents = "---\nid: c3d4\ntitle: Fix login bug\nstatus: open\nassignee:\n---\n";
    fs::write(issues.join(name), contents).unwrap();
    git(repo.path(), &["add", rel]);
    git(repo.path(), &["commit", "-m", "seed issue"]);

    // Change the user name to someone with no prior commits in this repo.
    git(repo.path(), &["config", "user.name", "Newcomer"]);
    git(repo.path(), &["config", "user.email", "newcomer@example.com"]);

    gi(repo.path())
        .args(["grab", "c3d4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Grabbed issue c3d4 for Newcomer"));

    let contents = fs::read_to_string(repo.path().join(rel)).unwrap();
    assert!(
        contents.contains("\nassignee: Newcomer\n"),
        "assignee not set:\n{contents}"
    );
    assert!(
        contents.contains("\nstatus: in_progress\n"),
        "status should be in_progress:\n{contents}"
    );
}

#[test]
fn unknown_id_fails_before_touching_anything() {
    let repo = init_repo();
    seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path())
        .args(["grab", "zzzz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue with id `zzzz`"));

    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after, before);
}

#[test]
fn grab_outside_a_git_repo_fails_cleanly() {
    let dir = TempDir::new().unwrap();
    gi(dir.path())
        .args(["grab", "a1b2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a git repository"));
}
