//! Black-box integration tests for `gi list` — the read path.
//!
//! These seed a throwaway temp repo (real `git`) with issue files spanning all
//! three states and assert on the rendered table for the default / `--all` /
//! `--done` views, plus the validation-on-read rejections (conflict markers,
//! unknown status). Assertions are on observable CLI output only.

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

/// Initialize a temp repo with a hermetic identity and signing disabled.
fn init_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "tester@example.com"]);
    git(dir.path(), &["config", "user.name", "Tester"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

/// Write an issue file directly under `.issues/`, controlling every field so
/// the rendered output is deterministic. `assignee` empty means unassigned.
fn seed(repo: &Path, hash: &str, title: &str, status: &str, assignee: &str) {
    let issues = repo.join(".issues");
    fs::create_dir_all(&issues).unwrap();
    let slug = title.to_lowercase().replace(' ', "-");
    let contents = format!(
        "---\nid: {hash}\ntitle: {title}\nstatus: {status}\nassignee: {assignee}\n---\nbody of {hash}\n"
    );
    fs::write(issues.join(format!("{slug}-{hash}.md")), contents).unwrap();
}

/// Write a raw `.issues/` file verbatim (for the malformed-input cases).
fn seed_raw(repo: &Path, filename: &str, contents: &str) {
    let issues = repo.join(".issues");
    fs::create_dir_all(&issues).unwrap();
    fs::write(issues.join(filename), contents).unwrap();
}

/// Build a `gi` command rooted in `repo` with a hermetic git environment.
fn gi(repo: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gi").unwrap();
    cmd.current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    cmd
}

/// Seed one issue in each state, plus a second open issue (unassigned) so the
/// `who` column exercises both an assignee and the `-` placeholder.
fn seed_across_states(repo: &Path) {
    seed(repo, "a1b2", "Fix login bug", "open", "joel");
    seed(repo, "9f2c", "Write docs", "open", ""); // unassigned -> `-`
    seed(repo, "7e3d", "Refactor parser", "in_progress", "ann");
    seed(repo, "c4d5", "Ship release", "done", "ann");
}

#[test]
fn default_shows_not_done_and_hides_done() {
    let repo = init_repo();
    seed_across_states(repo.path());

    gi(repo.path())
        .arg("list")
        .assert()
        .success()
        // Header and the not-done rows are present...
        .stdout(predicate::str::contains("hash"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("who"))
        .stdout(predicate::str::contains("title"))
        .stdout(predicate::str::contains("a1b2").and(predicate::str::contains("Fix login bug")))
        .stdout(predicate::str::contains("7e3d").and(predicate::str::contains("in_progress")))
        // Unassigned issue renders `-` in the who column...
        .stdout(predicate::str::contains("9f2c").and(predicate::str::contains("Write docs")))
        // ...and the done issue is hidden by default.
        .stdout(predicate::str::contains("Ship release").not())
        .stdout(predicate::str::contains("c4d5").not());
}

#[test]
fn all_includes_done_issues() {
    let repo = init_repo();
    seed_across_states(repo.path());

    gi(repo.path())
        .args(["list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix login bug"))
        .stdout(predicate::str::contains("Refactor parser"))
        .stdout(predicate::str::contains("Ship release").and(predicate::str::contains("done")))
        .stdout(predicate::str::contains("c4d5"));
}

#[test]
fn done_shows_only_done_issues() {
    let repo = init_repo();
    seed_across_states(repo.path());

    gi(repo.path())
        .args(["list", "--done"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Ship release"))
        .stdout(predicate::str::contains("c4d5"))
        // None of the not-done work appears.
        .stdout(predicate::str::contains("Fix login bug").not())
        .stdout(predicate::str::contains("Refactor parser").not());
}

#[test]
fn conflict_markers_are_rejected_on_read() {
    let repo = init_repo();
    seed(repo.path(), "a1b2", "Fine issue", "open", "joel");
    seed_raw(
        repo.path(),
        "half-merged-ffff.md",
        "---\nid: ffff\ntitle: Half merged\nstatus: open\n<<<<<<< HEAD\nassignee: a\n=======\nassignee: b\n>>>>>>> branch\n---\nbody\n",
    );

    gi(repo.path())
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("half-merged-ffff.md"))
        .stderr(predicate::str::contains("conflict markers"));
}

#[test]
fn unknown_status_is_reported_not_silently_shown() {
    let repo = init_repo();
    seed_raw(
        repo.path(),
        "bogus-eeee.md",
        "---\nid: eeee\ntitle: Bogus\nstatus: wat\nassignee:\n---\nbody\n",
    );

    gi(repo.path())
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("bogus-eeee.md"))
        .stderr(predicate::str::contains("unknown status"))
        // The bad issue is never rendered as if it were valid data.
        .stdout(predicate::str::contains("Bogus").not());
}

#[test]
fn empty_backlog_reports_no_issues() {
    let repo = init_repo();
    // No `.issues/` directory at all.
    gi(repo.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No issues."));
}

#[test]
fn all_and_done_are_mutually_exclusive() {
    let repo = init_repo();
    seed_across_states(repo.path());
    // clap rejects the conflicting flags before our code runs.
    gi(repo.path())
        .args(["list", "--all", "--done"])
        .assert()
        .failure();
}

#[test]
fn list_outside_a_git_repo_fails_cleanly() {
    let dir = TempDir::new().unwrap();
    gi(dir.path())
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a git repository"));
}
