# PRD: `gi` — a decentralized, git-native issue tracker

> Status: ready-for-agent
> Source: `README.md` (Design v1)

## Problem Statement

As a developer on a small team, my issues live in a centralized service (GitHub
Issues, Jira, Linear) that is separate from my code. This creates friction I feel
every day:

- I can't see or change issues offline — on a plane, on a bad connection, the
  tracker is gone but the code is right there.
- The issues for a commit aren't *in* the commit. History and intent drift apart;
  to understand why code changed I have to leave my editor and go correlate with a
  web app.
- Whoever owns the tracker owns my data. Migrating trackers, or just pulling all my
  issues out, is a project in itself.
- Onboarding a teammate means provisioning yet another account on yet another
  service, with its own permissions to manage.

I want my issues to travel with my code: anyone who can clone the repo has the
issues, full stop, with no extra service, account, or network call.

## Solution

`gi` is a single static binary that stores each issue as a markdown file under
`.issues/` on your working branch, committed alongside your code. There is no
server and no database — the git repository *is* the tracker.

From the user's perspective:

- Run `gi new` to create an issue. It writes a file, opens it in your `$EDITOR`,
  validates it on save, and commits it — all in one motion.
- Run `gi list` to see what's open and in progress, `gi board` to see the same
  thing as a read-only Kanban TUI.
- Move work along with three verbs that read like sentences: `gi assign <id> <who>`
  ("this is yours"), `gi grab <id>` ("I'm on it"), `gi done <id>` ("finished").
- Every change is an ordinary git commit, scoped to just that one issue file, so it
  rides your normal push/pull/merge flow and never sweeps up your other work.

Because issues are plain files merged line-by-line, two teammates working offline
never collide on creates, and most edits to different fields auto-merge. The cost
of decentralization (status is per-branch until merged; same-field edits can
conflict) is accepted and made visible rather than hidden.

## User Stories

### Creating issues

1. As a developer, I want to run `gi new` and have it create an issue file, so that
   I can capture a problem without leaving my terminal.
2. As a developer, I want `gi new` on a fresh repo to auto-create the `.issues/`
   directory, so that I don't have to scaffold anything before my first issue.
3. As a developer, I want `gi new` to open the new issue in my `$EDITOR`, so that I
   can write the title and body immediately while the thought is fresh.
4. As a developer, I want each new issue to get a short random hash id (e.g. `a1b2`),
   so that I have a stable, collision-free handle to reference it on the CLI.
5. As a developer, I want the issue filename to include a human-readable slug derived
   from the title, so that I can recognize issues when browsing `.issues/` directly.
6. As a developer creating an issue at the same time as a teammate, I want our new
   issues to land in different files, so that two creates never conflict in git.
7. As a developer, I want a new issue to start in the `open` state with me recorded
   as author, so that it shows up as actionable immediately.
8. As a developer, I want creating an issue to be committed automatically, so that
   capturing work and recording it in history are a single step.

### Listing and viewing issues

9. As a developer, I want `gi list` to show me not-done issues (open + in progress)
   by default, so that I see only what still needs attention.
10. As a developer, I want `gi list` to show `hash  status  who  title` columns, so
    that I can scan ownership and state at a glance.
11. As a developer, I want to pass `--all` to `gi list`, so that I can see closed
    issues alongside open ones when I need the full picture.
12. As a developer, I want to pass `--done` to `gi list`, so that I can review only
    completed work.
13. As a developer, I want to reference any issue by its short hash, so that I never
    have to type or paste a long identifier.

### Editing issues

14. As a developer, I want `gi edit <id>` to open the whole issue file (frontmatter +
    body) in my `$EDITOR`, so that I can change any field or the description in one
    place.
15. As a developer, I want my edits validated on save, so that I can't accidentally
    commit an issue with an unknown status or an unresolvable assignee.
16. As a developer, when my edit fails validation, I want the editor to reopen with
    an error message, so that I can fix the problem without losing my work.
17. As a developer, I want a valid edit to be committed automatically, so that my
    change is recorded the moment I save.

### Assigning and grabbing work

18. As a team lead, I want `gi assign <id> <who>` to set an issue's assignee while
    leaving it `open`, so that I can say "this is yours" without claiming it's
    started.
19. As a developer, I want `gi grab <id>` to assign the issue to me *and* move it to
    `in_progress`, so that one command says "I'm on it."
20. As a developer, I want to be able to grab or self-assign any issue, so that
    claiming work is never blocked by identity checks.
21. As a team lead, I want assignees validated against people who have actually
    committed to the repo, so that I can't typo a name onto an issue.
22. As a team lead, I want `assign` to accept either a name or an email and match it
    fuzzily to a known committer, so that I don't have to remember the exact form.
23. As a team lead, I want an `--force` escape hatch on `assign` (planned), so that I
    can assign someone who hasn't committed to the repo yet.

### Completing work

24. As a developer, I want `gi done <id>` to move an issue to `done`, so that I can
    close it out with one command.
25. As a developer, I want completing an issue to be committed automatically, so that
    the closure is recorded in history.

### The board

26. As a developer, I want `gi board` to render Open / In Progress / Done as three
    Kanban columns, so that I can see the state of all work at a glance.
27. As a developer, I want to scroll and select issues in the board, so that I can
    navigate a large backlog.
28. As a developer, I want to view an issue's detail from the board, so that I can
    read the body without dropping to the CLI.
29. As a developer, I want the board to be read-only, so that I never make an
    accidental mutation while browsing.

### Git behavior & collaboration

30. As a developer, I want every mutation committed with a clear message (e.g.
    `issue: close a1b2`), so that my git history explains itself.
31. As a developer, I want each issue commit scoped by pathspec to just that issue
    file, so that my other staged or in-flight work is never swept into an issue
    commit.
32. As a developer, I want `gi` to shell out to my own `git` binary, so that it
    inherits my config, credentials, hooks, and commit signing automatically.
33. As a developer pulling `main`, I want every issue created by my teammates to be
    present locally, so that the repo is the single source of truth for issues.
34. As two developers editing different fields of the same issue, we want git to
    auto-merge our changes, so that concurrent edits usually "just work."
35. As two developers editing the *same* field of an issue, we want a normal git
    conflict, so that the disagreement is surfaced and resolved by hand rather than
    silently lost.
36. As a developer, I want an issue file left with conflict markers to be caught on
    read, so that a half-merged issue can't masquerade as valid data.
37. As a developer, I want guidance to add `.issues/*.md text` to `.gitattributes`,
    so that issue merges stay line-based and predictable across platforms.

## Implementation Decisions

### Modules

- **`gi-core` (library):** all pure, effect-free logic — the issue model and
  frontmatter (de)serialization, slug + hash generation, the state machine, schema
  validation (including conflict-marker detection), and identity resolution. This is
  the unit-testable core and the secondary test seam.
- **`gi-cli` (binary):** argument parsing (the verbs in the CLI table), wiring
  `gi-core` to the real world, and the user-facing output for `list`. The
  black-box integration seam.
- **`board` (TUI module within the binary):** the read-only ratatui Kanban view.
  Built last; depends on `gi-core` for reading issues and on nothing else in the
  app. No other module depends on it.

### Effects behind thin traits

- **Editor effect:** opening `$EDITOR` is abstracted behind a small trait so the
  create/edit flows can be exercised without a real interactive editor. The
  default implementation launches `$EDITOR`.
- **Git effect:** committing is abstracted behind a small trait whose default
  implementation shells out to the user's `git` binary. *Note:* the git effect is
  verified end-to-end through the CLI seam against a real temp repo, not asserted via
  a mock — the trait exists for composability, not to replace real git in tests.

### Storage & schema (the data contract)

- **One file per issue:** `.issues/<slug>-<hash>.md`. Distinct files per issue is
  the mechanism that makes concurrent creates conflict-free.
- **Id = short random hash** (e.g. `a1b2`), the canonical CLI reference. The slug is
  cosmetic and derived from the title.
- **File format:** `---`-delimited YAML frontmatter, **one field per line** for
  merge-friendliness, followed by a markdown body. Frontmatter fields: `id`,
  `title`, `status`, `assignee`.
- **States:** `open → in_progress → done`. These are exactly the three board columns.
- **Single `assignee` field** drives two verbs (`assign`, `grab`) rather than
  separate ownership fields, keeping the schema minimal and merge-friendly.

### Verb semantics (the state transitions)

- `assign <id> <who>`: set `assignee`; `status` unchanged (stays `open`).
- `grab <id>`: set `assignee` to "me" **and** set `status = in_progress`.
- `done <id>`: set `status = done`.
- "me" is resolved from `git config user.email`.

### Identity resolution

- Valid assignees are derived from `git shortlog` authors, with fuzzy name↔email
  matching, **cached** to avoid re-deriving on every command.
- Self-assign and grab bypass the validity check and are always allowed.
- *Known limitation:* you cannot assign someone who has not yet committed; a planned
  `--force` flag on `assign` is the escape hatch.

### Git invocation

- Every mutation triggers an auto-commit scoped by pathspec to the single issue
  file, e.g. `git commit -m "issue: close a1b2" -- .issues/fix-login-bug-a1b2.md`.
- `gi` shells out to the user's `git` binary rather than linking a git library, so
  it inherits config, credentials, hooks, and signing.

### Validation-on-read

- Reading an issue validates its frontmatter (known status, resolvable assignee, no
  conflict markers). Invalid files surface an error rather than being silently
  treated as data — this is the safety net for hand-resolved merges.

### Defaults & flags

- `gi list` defaults to not-done (open + in progress); `--all` and `--done` widen
  the view.
- First `gi new` auto-creates `.issues/`.

## Testing Decisions

**What makes a good test here:** assert on *external, observable behavior*, never on
internal structure. For this tool the observable surface is (a) the bytes in
`.issues/*.md` after a command runs and (b) the resulting `git log` / committed
state. Tests should never reach into private functions to assert intermediate state
when the same property is observable at the CLI boundary.

### Primary seam — `gi-cli` black-box integration (the highest seam)

- Drive the compiled binary (`assert_cmd`) against a throwaway temp repo
  (`tempfile`) initialized with a **real `git`** binary.
- Cover the full verb set end-to-end: `new` (incl. first-run `.issues/` creation),
  `list` (default vs `--all`/`--done`), `edit`, `assign`, `grab`, `done`.
- Assert on observable outcomes only: the issue files on disk and the commits in
  `git log` (message, and that the pathspec scoped the commit to exactly one file —
  e.g. an unrelated dirty/staged file is *not* swept into an issue commit).
- The `$EDITOR` interaction is exercised by pointing `$EDITOR`/`$VISUAL` (or the
  editor trait) at a scripted non-interactive stand-in that writes known content,
  so create/edit flows run unattended.
- Using real git is deliberate: the tool's contract is "shell out to the user's git,"
  so faithful tests use real git rather than a fake. (Confirmed with the developer.)

### Secondary seam — `gi-core` unit tests

- Frontmatter round-trip (serialize → parse) and schema validation, including the
  conflict-marker rejection path.
- Slug + hash generation (slug derives correctly from titles; hashes are
  well-formed and unique within a run).
- State machine: each verb produces the correct `(status, assignee)` transition;
  invalid transitions/states are rejected.
- Identity resolution: shortlog-author parsing and fuzzy name↔email matching against
  a fixed fixture of committers; self-assign/grab always allowed.

### Deferred seam — `board` TUI

- Minimal coverage via ratatui's `TestBackend` buffer snapshots (issues land in the
  correct column; detail view renders a selected issue). Read-only and built last,
  so it carries the lightest test burden.

### Prior art

- This is a greenfield Rust project, so there is no in-repo prior art yet. The
  intended idioms are the standard Rust testing stack: `assert_cmd` + `predicates` +
  `tempfile` for CLI integration, ordinary `#[test]` unit tests in `gi-core`, and
  `ratatui::backend::TestBackend` for the TUI. The first slice that establishes the
  CLI seam becomes the prior art for every subsequent slice.

## Out of Scope

- **Any centralized component:** no server, daemon, sync service, or web UI.
- **Mutations from the board:** `gi board` is strictly read-only in v1; all changes
  go through the CLI verbs.
- **The `--force` assign flag:** acknowledged as planned, but not part of v1.
- **Issue relationships:** no labels, milestones, priorities, comments threads,
  parent/child links, or cross-references between issues in v1. The schema is `id`,
  `title`, `status`, `assignee`, body.
- **Automatic `.gitattributes` management:** `gi` *recommends* adding
  `.issues/*.md text`; it does not write or enforce it.
- **Automated merge-conflict resolution:** conflicts are surfaced (git markers +
  validation-on-read) and resolved by hand; `gi` does not attempt to auto-resolve.
- **Cross-branch status reconciliation:** an issue closed on a feature branch is not
  closed elsewhere until that branch merges. This is an accepted tradeoff, not a
  problem to solve in v1.
- **Pushing/pulling on behalf of the user:** `gi` commits; it does not auto-push or
  fetch. Distribution rides the user's normal git workflow.

## Further Notes

- **Accepted tradeoffs of same-branch storage** (from the design): status changes
  are commits on your branch; closure is per-branch until merge; issue churn
  interleaves with code history. These are deliberate consequences of "issues travel
  with code," documented so they aren't mistaken for bugs.
- **Build order:** `gi-core` first (it has the testable contract and no
  dependencies), then `gi-cli` verbs on top, then `board` last since nothing depends
  on it. This order also front-loads the highest-value test seam.
- **Single static binary** is a hard product constraint — `gi` must be droppable into
  a repo/CI with no runtime dependencies beyond the user's existing `git`.
- **`gi` is intended to eventually self-host its own issues** in `.issues/`, but until
  the binary exists this PRD lives at `docs/prd-gi.md` (no tracker was configured for
  this session).
