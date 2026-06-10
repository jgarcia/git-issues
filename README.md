# Git Issues

A decentralized issue tracker. Instead of a centralized service holding all the
issues, issues are files committed alongside your code — so anyone who pulls main
has the issues.

## Features

- List open issues
- Create a new issue
- Open an issue in the editor of your choice via `$EDITOR`
- Mark an issue as complete
- Assign an issue to a team member
- Grab an issue to work on it
- Read-only Kanban TUI

## Design (v1)

### Storage

- **One file per issue:** `.issues/<slug>-<hash>.md` (e.g. `fix-login-bug-a1b2.md`).
  Two people creating issues never touch the same file, so creates never conflict.
- **ID = short random hash** (`a1b2`) — the canonical reference used on the CLI.
  Collision-free for teammates working offline. The slug is for human readability.
- **Same branch as code:** issue files live in `.issues/` on your working branch.
  Accepted tradeoffs: status changes are commits on your branch; an issue closed on
  a feature branch isn't closed everywhere until that branch merges; issue churn
  interleaves with code history.
- **Format:** `---` YAML frontmatter (one field per line, for merge-friendliness)
  followed by a markdown body.

```
.issues/
  fix-login-bug-a1b2.md
  add-logout-9f2k.md

--- fix-login-bug-a1b2.md ---
---
id: a1b2
title: Fix login bug
status: open
assignee: joel
---
The login button does nothing when...
```

### Lifecycle & schema

- **States:** `open → in_progress → done` — the three Kanban columns.
- **Single `assignee` field**, two verbs:
  - `assign <id> <who>` sets the assignee; status stays `open` ("this is yours").
  - `grab <id>` self-assigns **and** flips status to `in_progress` ("I'm on it").
  - `done <id>` moves the issue to `done`.
- **Identity:** "me" = `git config user.email`. Valid assignees are derived from
  `git shortlog` authors (with fuzzy name↔email matching, cached). Self-assign /
  grab is always allowed.
  - *Limitation:* you can't assign someone who hasn't committed yet. An `--force`
    escape hatch on `assign` is planned for this case.

### Git behavior

- **Auto-commit on every mutation**, scoped by pathspec so your other staged or
  in-flight work is never swept in:

  ```
  git commit -m "issue: close a1b2" -- .issues/fix-login-bug-a1b2.md
  ```

- **Shells out to the user's `git` binary** — inherits their config, credentials,
  hooks, and commit signing.
- **Concurrent same-issue edits:** rely on git's line-based merge. Edits to
  different fields auto-merge; same-field edits (e.g. both set `assignee`) conflict
  and are resolved by hand. Validation-on-read catches any file left with conflict
  markers.
- Add `.issues/*.md text` to `.gitattributes` to keep merges line-based across
  platforms.

### CLI

`gi` — a single static binary (Rust + [ratatui](https://ratatui.rs)).

| Command            | Behavior                                                              |
| ------------------ | -------------------------------------------------------------------- |
| `gi new`           | Create an issue; first run auto-creates `.issues/`.                  |
| `gi list`          | List **not-done** issues (Open + In Progress) by default.            |
| `gi edit <id>`     | Open the whole file in `$EDITOR`; validate on save, then commit.    |
| `gi done <id>`     | Mark complete.                                                       |
| `gi assign <id> <who>` | Assign to a team member.                                         |
| `gi grab <id>`     | Self-assign and move to In Progress.                                 |
| `gi board`         | Read-only Kanban TUI.                                                |

- `gi list` columns: `hash  status  who  title`. Use `--all` or `--done` to see
  closed issues.
- `gi edit` opens frontmatter + body; on save the frontmatter is validated (known
  status, resolvable assignee) and the editor reopens with an error if invalid,
  then the file is committed.
- `gi board` renders the three columns from `.issues/` and lets you scroll, select,
  and view detail. All mutations happen via the CLI verbs. Built last — nothing
  else depends on it.
