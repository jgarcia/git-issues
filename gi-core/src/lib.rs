//! `gi-core` — the pure, effect-free heart of the `gi` issue tracker.
//!
//! This crate owns the issue model and its on-disk representation: the
//! one-field-per-line YAML frontmatter format, slug + short-hash generation,
//! and the lifecycle states. It performs **no** I/O of its own beyond reading
//! the system entropy source for hash generation — opening editors, touching
//! the filesystem and committing to git are effects that live in `gi-cli`.

use std::fmt;
use std::fs::File;
use std::io::Read;

/// The lifecycle state of an issue — the three Kanban columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open,
    InProgress,
    Done,
}

impl Status {
    /// The canonical on-disk / on-CLI string for this status.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::InProgress => "in_progress",
            Status::Done => "done",
        }
    }

    /// Sort key reflecting workflow order: `open` before `in_progress` before
    /// `done`. Used to give `gi list` a stable, intuitive ordering.
    pub fn rank(self) -> u8 {
        match self {
            Status::Open => 0,
            Status::InProgress => 1,
            Status::Done => 2,
        }
    }

    /// Parse a status from its canonical string. Unknown values yield `None`.
    pub fn parse(s: &str) -> Option<Status> {
        match s.trim() {
            "open" => Some(Status::Open),
            "in_progress" => Some(Status::InProgress),
            "done" => Some(Status::Done),
            _ => None,
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single issue: frontmatter fields plus the markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub status: Status,
    /// `None` means unassigned. A freshly created issue is unassigned; the git
    /// commit author is the record of who created it.
    pub assignee: Option<String>,
    pub body: String,
}

/// Why a stored issue failed to parse. Surfaced to the user on read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    MissingFrontmatter,
    UnterminatedFrontmatter,
    MissingField(&'static str),
    UnknownStatus(String),
    ConflictMarkers,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::MissingFrontmatter => {
                write!(f, "missing `---` frontmatter header")
            }
            ParseError::UnterminatedFrontmatter => {
                write!(f, "frontmatter is not terminated by a closing `---`")
            }
            ParseError::MissingField(name) => write!(f, "missing required field `{name}`"),
            ParseError::UnknownStatus(s) => write!(f, "unknown status `{s}`"),
            ParseError::ConflictMarkers => {
                write!(f, "file contains unresolved git conflict markers")
            }
        }
    }
}

impl std::error::Error for ParseError {}

impl Issue {
    /// Create a fresh, unassigned, open issue with the given id and title.
    pub fn create(id: impl Into<String>, title: impl Into<String>) -> Issue {
        Issue {
            id: id.into(),
            title: title.into(),
            status: Status::Open,
            assignee: None,
            body: String::new(),
        }
    }

    /// Serialize to the on-disk file format: `---` frontmatter (one field per
    /// line, fixed order for merge-friendliness) followed by the body.
    pub fn to_file_string(&self) -> String {
        let assignee_line = match &self.assignee {
            Some(a) if !a.is_empty() => format!("assignee: {a}"),
            _ => "assignee:".to_string(),
        };
        format!(
            "---\nid: {}\ntitle: {}\nstatus: {}\n{}\n---\n{}",
            self.id,
            self.title,
            self.status.as_str(),
            assignee_line,
            self.body,
        )
    }

    /// Parse an issue from its on-disk representation, validating as it goes.
    /// This is the read-side gate: unknown status, missing fields, or leftover
    /// conflict markers are reported rather than silently accepted.
    pub fn parse(contents: &str) -> Result<Issue, ParseError> {
        if has_conflict_markers(contents) {
            return Err(ParseError::ConflictMarkers);
        }

        let mut lines = contents.lines();
        match lines.next() {
            Some(l) if l.trim_end() == "---" => {}
            _ => return Err(ParseError::MissingFrontmatter),
        }

        let mut id = None;
        let mut title = None;
        let mut status_raw = None;
        let mut assignee = None;
        let mut terminated = false;

        for line in lines.by_ref() {
            if line.trim_end() == "---" {
                terminated = true;
                break;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim().to_string();
            match key.trim() {
                "id" => id = Some(value),
                "title" => title = Some(value),
                "status" => status_raw = Some(value),
                "assignee" => assignee = Some(value),
                _ => {}
            }
        }

        if !terminated {
            return Err(ParseError::UnterminatedFrontmatter);
        }

        let id = id
            .filter(|s| !s.is_empty())
            .ok_or(ParseError::MissingField("id"))?;
        let title = title
            .filter(|s| !s.is_empty())
            .ok_or(ParseError::MissingField("title"))?;
        let status_raw = status_raw.ok_or(ParseError::MissingField("status"))?;
        let status =
            Status::parse(&status_raw).ok_or(ParseError::UnknownStatus(status_raw.clone()))?;
        let assignee = assignee.filter(|s| !s.is_empty());

        // Everything after the closing `---` is the body.
        let body = remainder_after_frontmatter(contents);

        Ok(Issue {
            id,
            title,
            status,
            assignee,
            body,
        })
    }

    /// The filename this issue lives under: `<slug>-<hash>.md`.
    pub fn filename(&self) -> String {
        format!("{}-{}.md", slugify(&self.title), self.id)
    }
}

/// Return the body: everything following the second `---` delimiter line.
fn remainder_after_frontmatter(contents: &str) -> String {
    let mut seen = 0;
    let mut idx = 0;
    for line in contents.split_inclusive('\n') {
        idx += line.len();
        if line.trim_end() == "---" {
            seen += 1;
            if seen == 2 {
                break;
            }
        }
    }
    contents[idx..].to_string()
}

/// True if the text contains git conflict markers at the start of any line.
fn has_conflict_markers(contents: &str) -> bool {
    contents
        .lines()
        .any(|l| l.starts_with("<<<<<<<") || l.starts_with("=======") || l.starts_with(">>>>>>>"))
}

/// Derive a human-readable, filesystem-safe slug from an issue title.
///
/// Lowercases, replaces any run of non-`[a-z0-9]` characters with a single
/// hyphen, trims leading/trailing hyphens, and caps the length. An empty or
/// punctuation-only title slugs to `issue`.
pub fn slugify(title: &str) -> String {
    const MAX: usize = 50;
    let mut out = String::with_capacity(title.len().min(MAX));
    let mut prev_hyphen = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if !prev_hyphen && !out.is_empty() {
            out.push('-');
            prev_hyphen = true;
        }
    }
    while out.len() > MAX {
        out.pop();
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "issue".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Generate a short random hash (4 hex chars) for use as an issue id.
///
/// Reads two bytes from the system entropy source. Distinct files per issue
/// make creates conflict-free, so a tiny id space is fine for a small team;
/// the caller retries on the rare collision with an existing file.
pub fn new_hash() -> String {
    let mut buf = [0u8; 2];
    if File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_err()
    {
        // Fallback for platforms without /dev/urandom: mix clock and pid.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let mixed = nanos ^ std::process::id();
        buf[0] = mixed as u8;
        buf[1] = (mixed >> 8) as u8;
    }
    format!("{:02x}{:02x}", buf[0], buf[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_round_trips() {
        let issue = Issue {
            id: "a1b2".to_string(),
            title: "Fix login bug".to_string(),
            status: Status::InProgress,
            assignee: Some("joel".to_string()),
            body: "The login button does nothing when clicked.\n".to_string(),
        };
        let parsed = Issue::parse(&issue.to_file_string()).expect("parse");
        assert_eq!(parsed, issue);
    }

    #[test]
    fn round_trips_when_unassigned_with_empty_body() {
        let issue = Issue::create("9f2k", "Add logout");
        let parsed = Issue::parse(&issue.to_file_string()).expect("parse");
        assert_eq!(parsed, issue);
        assert_eq!(parsed.assignee, None);
        assert_eq!(parsed.status, Status::Open);
    }

    #[test]
    fn fresh_issue_is_open_and_unassigned() {
        let issue = Issue::create("c3d4", "Anything");
        assert_eq!(issue.status, Status::Open);
        assert_eq!(issue.assignee, None);
    }

    #[test]
    fn unknown_status_is_rejected() {
        let raw = "---\nid: a1b2\ntitle: x\nstatus: wat\nassignee:\n---\n";
        assert_eq!(
            Issue::parse(raw),
            Err(ParseError::UnknownStatus("wat".to_string()))
        );
    }

    #[test]
    fn status_rank_orders_open_then_in_progress_then_done() {
        assert!(Status::Open.rank() < Status::InProgress.rank());
        assert!(Status::InProgress.rank() < Status::Done.rank());
    }

    #[test]
    fn missing_header_is_rejected() {
        assert_eq!(
            Issue::parse("id: a1b2\n"),
            Err(ParseError::MissingFrontmatter)
        );
    }

    #[test]
    fn conflict_markers_are_rejected() {
        let raw = "---\nid: a1b2\ntitle: x\nstatus: open\n<<<<<<< HEAD\nassignee: a\n=======\nassignee: b\n>>>>>>> branch\n---\n";
        assert_eq!(Issue::parse(raw), Err(ParseError::ConflictMarkers));
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Fix login bug"), "fix-login-bug");
    }

    #[test]
    fn slugify_collapses_and_trims_punctuation() {
        assert_eq!(slugify("  Add   logout!! (please) "), "add-logout-please");
    }

    #[test]
    fn slugify_empty_title_falls_back() {
        assert_eq!(slugify("!!!"), "issue");
        assert_eq!(slugify(""), "issue");
    }

    #[test]
    fn slugify_caps_length_without_trailing_hyphen() {
        let long = "a ".repeat(80);
        let slug = slugify(&long);
        assert!(slug.len() <= 50);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn hash_is_four_hex_chars() {
        let h = new_hash();
        assert_eq!(h.len(), 4);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hashes_are_generally_distinct() {
        // Not a guarantee, but a 2-byte space should not collide across a
        // handful of draws in practice.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            seen.insert(new_hash());
        }
        assert!(seen.len() > 40, "too many hash collisions: {}", seen.len());
    }

    #[test]
    fn filename_combines_slug_and_id() {
        let issue = Issue::create("a1b2", "Fix login bug");
        assert_eq!(issue.filename(), "fix-login-bug-a1b2.md");
    }
}
