//! Black-box integration tests for `gi edit` — the whole-file edit with a
//! validate-on-save loop.
//!
//! Like the other suites, these drive the compiled `gi` binary against a
//! throwaway temp repo backed by a *real* `git`, with `$EDITOR` pointed at a
//! scripted, non-interactive stand-in. The stand-ins simulate the user's edits
//! (and, for the reopen loop, behave differently across successive opens), so
//! the validate → reopen-with-error → revalidate flow runs unattended.
//! Assertions are on observable outcomes only: the issue file and the `git log`.

use std::fs;
use std::path::{Path, PathBuf};
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

/// Build a `gi` command rooted in `repo`, with `$EDITOR`/`$VISUAL` wired to the
/// given stand-in and a hermetic git environment.
fn gi(repo: &Path, editor: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gi").unwrap();
    cmd.current_dir(repo)
        .env("EDITOR", editor)
        .env("VISUAL", editor)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    cmd
}

/// Write an executable shell script (the editor stand-in) with the given body,
/// which receives the file to edit as `$1`.
fn write_editor(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// Seed an issue file under `.issues/` and commit it (as `gi new` would leave
/// it). Returns the file's name and relative path.
fn seed_committed_issue(repo: &Path, hash: &str, title: &str, status: &str) -> (String, String) {
    let issues = repo.join(".issues");
    fs::create_dir_all(&issues).unwrap();
    let slug = title.to_lowercase().replace(' ', "-");
    let name = format!("{slug}-{hash}.md");
    let rel = format!(".issues/{name}");
    let contents = format!(
        "---\nid: {hash}\ntitle: {title}\nstatus: {status}\nassignee:\n---\nbody of {hash}\n"
    );
    fs::write(issues.join(&name), contents).unwrap();
    git(repo, &["add", &rel]);
    git(repo, &["commit", "-m", "seed issue"]);
    (name, rel)
}

#[test]
fn valid_edit_is_normalized_and_committed_scoped_to_that_file() {
    let repo = init_repo();
    let (_name, rel) = seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    // A stand-in that flips status to in_progress and adds a line to the body.
    let editor = write_editor(
        repo.path(),
        "valid-editor.sh",
        "f=\"$1\"\n\
         tmp=\"$f.tmp\"\n\
         sed 's/^status: .*/status: in_progress/' \"$f\" > \"$tmp\" && mv \"$tmp\" \"$f\"\n\
         printf '%s\\n' 'EDITED_BODY_MARKER' >> \"$f\"\n",
    );

    // Unrelated staged work must not be swept into the edit commit.
    fs::write(repo.path().join("unrelated.txt"), "do not commit me\n").unwrap();
    git(repo.path(), &["add", "unrelated.txt"]);

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path(), &editor)
        .args(["edit", "a1b2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Edited issue a1b2"));

    // Both the field change and the body edit landed.
    let contents = fs::read_to_string(repo.path().join(&rel)).unwrap();
    assert!(
        contents.contains("\nstatus: in_progress\n"),
        "status not updated:\n{contents}"
    );
    assert!(
        contents.contains("EDITED_BODY_MARKER"),
        "body edit not saved:\n{contents}"
    );

    // Exactly one new commit, whose subject names the id.
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after.parse::<u32>().unwrap(), before.parse::<u32>().unwrap() + 1);
    assert_eq!(
        git_out(repo.path(), &["log", "-1", "--pretty=%s"]),
        "issue: edit a1b2"
    );

    // That commit holds ONLY the issue file — unrelated.txt is left staged.
    let committed = git_out(
        repo.path(),
        &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
    );
    assert_eq!(committed, rel);
    assert_eq!(
        git_out(repo.path(), &["diff", "--cached", "--name-only"]),
        "unrelated.txt"
    );
}

#[test]
fn invalid_save_reopens_with_error_preserving_edits_then_commits_the_fix() {
    let repo = init_repo();
    let (_name, rel) = seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    // A counter-driven stand-in: the first save introduces an unknown status; on
    // the reopen it records what it was shown (to prove the banner + the user's
    // bad edit survived), then fixes the status to a valid value.
    let editor = write_editor(
        repo.path(),
        "loop-editor.sh",
        "f=\"$1\"\n\
         d=$(dirname \"$0\")\n\
         c=\"$d/edit-count\"\n\
         n=$(cat \"$c\" 2>/dev/null || echo 0)\n\
         n=$((n + 1))\n\
         printf '%s' \"$n\" > \"$c\"\n\
         tmp=\"$f.tmp\"\n\
         if [ \"$n\" -eq 1 ]; then\n\
           sed 's/^status: .*/status: bogus/' \"$f\" > \"$tmp\" && mv \"$tmp\" \"$f\"\n\
         else\n\
           cp \"$f\" \"$d/reopened.txt\"\n\
           sed 's/^status: .*/status: in_progress/' \"$f\" > \"$tmp\" && mv \"$tmp\" \"$f\"\n\
         fi\n",
    );

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path(), &editor)
        .args(["edit", "a1b2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Edited issue a1b2"));

    // The editor was reopened exactly twice (one invalid save, one fix).
    assert_eq!(
        fs::read_to_string(repo.path().join("edit-count")).unwrap(),
        "2"
    );

    // What the second open was shown: the error banner naming the bad status, and
    // the user's in-progress (invalid) edit, both preserved.
    let reopened = fs::read_to_string(repo.path().join("reopened.txt")).unwrap();
    assert!(
        reopened.contains("# gi:") && reopened.contains("unknown status `bogus`"),
        "reopen did not show the error banner:\n{reopened}"
    );
    assert!(
        reopened.contains("status: bogus"),
        "reopen did not preserve the user's edit:\n{reopened}"
    );

    // The committed file is the fixed, normalized version: no banner, no bad value.
    let contents = fs::read_to_string(repo.path().join(&rel)).unwrap();
    assert!(contents.starts_with("---\n"), "banner leaked into file:\n{contents}");
    assert!(!contents.contains("# gi:"), "banner leaked into file:\n{contents}");
    assert!(
        contents.contains("\nstatus: in_progress\n") && !contents.contains("bogus"),
        "fix not saved:\n{contents}"
    );

    // Exactly one new commit despite the reopen loop.
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after.parse::<u32>().unwrap(), before.parse::<u32>().unwrap() + 1);
    assert_eq!(
        git_out(repo.path(), &["log", "-1", "--pretty=%s"]),
        "issue: edit a1b2"
    );
}

#[test]
fn unresolvable_assignee_is_rejected_and_makes_no_commit() {
    let repo = init_repo();
    let (_name, rel) = seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    // A stand-in that always sets an assignee nobody in this repo's history can
    // resolve. It makes the same edit each time, so the reopen leaves the file
    // unchanged — exercising both the assignee check and the no-progress guard
    // that stops `gi edit` from looping forever against a non-interactive editor.
    let editor = write_editor(
        repo.path(),
        "bad-assignee-editor.sh",
        "f=\"$1\"\n\
         tmp=\"$f.tmp\"\n\
         sed 's/^assignee:.*/assignee: ghost/' \"$f\" > \"$tmp\" && mv \"$tmp\" \"$f\"\n",
    );

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path(), &editor)
        .args(["edit", "a1b2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("matches no committer"));

    // No new commit, and the file keeps the user's edit (no leftover banner).
    let after = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(after, before);
    let contents = fs::read_to_string(repo.path().join(&rel)).unwrap();
    assert!(!contents.contains("# gi:"), "banner left behind:\n{contents}");
    assert!(
        contents.contains("assignee: ghost"),
        "user's edit was discarded:\n{contents}"
    );
}

#[test]
fn no_op_save_makes_no_commit() {
    let repo = init_repo();
    seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    // A stand-in that leaves the file untouched.
    let editor = write_editor(repo.path(), "noop-editor.sh", "exit 0\n");

    let before = git_out(repo.path(), &["rev-list", "--count", "HEAD"]);

    gi(repo.path(), &editor)
        .args(["edit", "a1b2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unchanged"));

    // Nothing was committed for an edit that changed nothing.
    assert_eq!(git_out(repo.path(), &["rev-list", "--count", "HEAD"]), before);
}

#[test]
fn unknown_id_fails_before_opening_the_editor() {
    let repo = init_repo();
    seed_committed_issue(repo.path(), "a1b2", "Fix login bug", "open");

    // An editor that would error if ever run, proving we never reach it.
    let editor = write_editor(repo.path(), "explode-editor.sh", "exit 7\n");

    gi(repo.path(), &editor)
        .args(["edit", "zzzz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue with id `zzzz`"));
}

#[test]
fn edit_outside_a_git_repo_fails_cleanly() {
    let dir = TempDir::new().unwrap();
    let editor = write_editor(dir.path(), "noop-editor.sh", "exit 0\n");

    gi(dir.path(), &editor)
        .args(["edit", "a1b2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a git repository"));
}
