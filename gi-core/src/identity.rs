//! Identity resolution: turning a loose `<who>` argument into a known
//! committer.
//!
//! The set of valid assignees is *derived*, not configured: it is exactly the
//! people who have authored commits in the repository, read off `git shortlog`.
//! This module is pure — it parses shortlog text and matches a query against the
//! parsed set. Running git and caching the result are effects that live in
//! `gi-cli`; here we only need the bytes git produced.

/// One author from the repository's history: a display name and an email.
///
/// Either field may be empty in pathological history (a commit with no name, or
/// an `<>` email), but at least one is always present for a parsed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committer {
    pub name: String,
    pub email: String,
}

impl Committer {
    /// How this committer should be written into an issue's `assignee` field:
    /// the name when there is one, otherwise the email as a fallback so the
    /// stored value is never empty.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.email
        } else {
            &self.name
        }
    }
}

/// The outcome of resolving a `<who>` query against the committer set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<'a> {
    /// Exactly one committer matched.
    Matched(&'a Committer),
    /// Several committers matched the query; the caller must disambiguate.
    Ambiguous(Vec<&'a Committer>),
    /// No committer matched — the query resolves to nobody in this history.
    NotFound,
}

/// Parse the output of `git shortlog -sne` into committers.
///
/// Each summary line looks like `␣␣␣␣42␉Ada Lovelace <ada@example.com>`: leading
/// whitespace, a commit count, a tab, then `Name <email>`. We strip the count
/// and split the name from the angle-bracketed email. Lines without an email are
/// skipped rather than guessed at.
pub fn parse_shortlog(output: &str) -> Vec<Committer> {
    output.lines().filter_map(parse_shortlog_line).collect()
}

fn parse_shortlog_line(line: &str) -> Option<Committer> {
    // Drop the leading count and the whitespace around it, leaving `Name <email>`.
    let rest = line.trim_start_matches(|c: char| c.is_ascii_digit() || c.is_whitespace());

    // The email is the last `<...>` span; everything before it is the name.
    let open = rest.rfind('<')?;
    let close = rest[open..].find('>')? + open;
    let name = rest[..open].trim().to_string();
    let email = rest[open + 1..close].trim().to_string();

    if name.is_empty() && email.is_empty() {
        return None;
    }
    Some(Committer { name, email })
}

/// Resolve a free-form `<who>` to a single committer, fuzzily.
///
/// Matching is case-insensitive and works against either the name or the email:
///
/// 1. An exact (case-insensitive) match on a full name or email wins outright,
///    so `ada@example.com` or `Ada Lovelace` pins the person even if a shorter
///    query would have been ambiguous.
/// 2. Otherwise any committer whose name or email *contains* the query matches —
///    so `ada` finds `Ada Lovelace` and `ada@example.com` alike. One hit
///    resolves; several is reported as ambiguous rather than guessed.
///
/// An empty query, or one that matches nobody, yields [`Resolution::NotFound`].
pub fn resolve<'a>(who: &str, committers: &'a [Committer]) -> Resolution<'a> {
    let q = who.trim().to_lowercase();
    if q.is_empty() {
        return Resolution::NotFound;
    }

    // Exact name/email match takes precedence over any substring matches.
    if let Some(c) = committers
        .iter()
        .find(|c| c.name.to_lowercase() == q || c.email.to_lowercase() == q)
    {
        return Resolution::Matched(c);
    }

    let hits: Vec<&Committer> = committers
        .iter()
        .filter(|c| c.name.to_lowercase().contains(&q) || c.email.to_lowercase().contains(&q))
        .collect();

    match hits.len() {
        0 => Resolution::NotFound,
        1 => Resolution::Matched(hits[0]),
        _ => Resolution::Ambiguous(hits),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed committer fixture, as `git shortlog -sne` would print it.
    const SHORTLOG: &str = "\
    42\tAda Lovelace <ada@example.com>
     7\tGrace Hopper <grace@example.com>
     3\talan <alan@bletchley.uk>
";

    fn fixture() -> Vec<Committer> {
        parse_shortlog(SHORTLOG)
    }

    #[test]
    fn parses_count_name_and_email() {
        let committers = fixture();
        assert_eq!(
            committers,
            vec![
                Committer {
                    name: "Ada Lovelace".into(),
                    email: "ada@example.com".into()
                },
                Committer {
                    name: "Grace Hopper".into(),
                    email: "grace@example.com".into()
                },
                Committer {
                    name: "alan".into(),
                    email: "alan@bletchley.uk".into()
                },
            ]
        );
    }

    #[test]
    fn skips_lines_without_an_email() {
        let committers = parse_shortlog("    9\tNo Email Here\n   42\tAda <ada@x.io>\n");
        assert_eq!(committers.len(), 1);
        assert_eq!(committers[0].email, "ada@x.io");
    }

    #[test]
    fn resolves_exact_name_case_insensitively() {
        let c = fixture();
        assert_eq!(resolve("ada lovelace", &c), Resolution::Matched(&c[0]));
    }

    #[test]
    fn resolves_exact_email() {
        let c = fixture();
        assert_eq!(resolve("grace@example.com", &c), Resolution::Matched(&c[1]));
    }

    #[test]
    fn resolves_fuzzy_first_name() {
        let c = fixture();
        assert_eq!(resolve("grace", &c), Resolution::Matched(&c[1]));
    }

    #[test]
    fn resolves_fuzzy_email_local_part() {
        let c = fixture();
        assert_eq!(resolve("bletchley", &c), Resolution::Matched(&c[2]));
    }

    #[test]
    fn exact_match_beats_a_substring_collision() {
        // "alan" is an exact name for committer 2, but also a substring of an
        // email below; the exact name must win rather than report ambiguity.
        let c = vec![
            Committer {
                name: "alan".into(),
                email: "alan@bletchley.uk".into(),
            },
            Committer {
                name: "Bob".into(),
                email: "alan-fan@example.com".into(),
            },
        ];
        assert_eq!(resolve("alan", &c), Resolution::Matched(&c[0]));
    }

    #[test]
    fn ambiguous_query_lists_all_hits() {
        let c = fixture();
        // "example.com" is a substring of two emails.
        match resolve("example.com", &c) {
            Resolution::Ambiguous(hits) => assert_eq!(hits.len(), 2),
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn unknown_who_resolves_to_nobody() {
        let c = fixture();
        assert_eq!(resolve("nobody", &c), Resolution::NotFound);
        assert_eq!(resolve("   ", &c), Resolution::NotFound);
    }

    #[test]
    fn label_falls_back_to_email_when_nameless() {
        let c = Committer {
            name: String::new(),
            email: "ghost@example.com".into(),
        };
        assert_eq!(c.label(), "ghost@example.com");
    }
}
