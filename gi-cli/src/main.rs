//! `gi` — a decentralized, git-native issue tracker.
//!
//! Issues are markdown files under `.issues/`, committed alongside your code.
//! This binary is the CLI surface; the pure issue model lives in `gi-core`.

mod commands;
mod effects;

use anyhow::Result;
use clap::{Parser, Subcommand};

use effects::{SystemEditor, SystemGit};

#[derive(Parser)]
#[command(
    name = "gi",
    version,
    about = "A decentralized, git-native issue tracker"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new issue and open it in your editor.
    New {
        /// The issue title (the rest of the line). A slug is derived from it.
        #[arg(required = true, num_args = 1..)]
        title: Vec<String>,
    },
    /// List issues. Defaults to not-done (open + in progress).
    List {
        /// Include done issues alongside open and in-progress ones.
        #[arg(long, conflicts_with = "done")]
        all: bool,
        /// Show only done issues.
        #[arg(long)]
        done: bool,
    },
    /// Open the whole issue file in your editor; validate on save, then commit.
    Edit {
        /// The id (short hash) of the issue to edit.
        id: String,
    },
    /// Assign an issue to a team member, leaving its status unchanged.
    Assign {
        /// The id (short hash) of the issue to assign.
        id: String,
        /// Who to assign it to: a name or email, matched fuzzily against the
        /// repo's committers.
        who: String,
    },
    /// Close an issue: set its status to done and commit the change.
    Done {
        /// The id (short hash) of the issue to close.
        id: String,
    },
    /// Self-assign an issue and move it to in-progress.
    Grab {
        /// The id (short hash) of the issue to grab.
        id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;

    match cli.command {
        Command::New { title } => {
            let title = title.join(" ");
            let created = commands::new(&SystemEditor, &SystemGit, &cwd, &title)?;
            println!("Created issue {} ({})", created.id, created.rel_path);
        }
        Command::List { all, done } => {
            let filter = if done {
                commands::Filter::Done
            } else if all {
                commands::Filter::All
            } else {
                commands::Filter::NotDone
            };
            let table = commands::list(&SystemGit, &cwd, filter)?;
            println!("{table}");
        }
        Command::Edit { id } => {
            let edited = commands::edit(&SystemEditor, &SystemGit, &cwd, &id)?;
            if edited.unchanged {
                println!("Issue {} unchanged ({})", edited.id, edited.rel_path);
            } else {
                println!("Edited issue {} ({})", edited.id, edited.rel_path);
            }
        }
        Command::Assign { id, who } => {
            let assigned = commands::assign(&SystemGit, &cwd, &id, &who)?;
            println!(
                "Assigned issue {} to {} ({})",
                assigned.id, assigned.assignee, assigned.rel_path
            );
        }
        Command::Done { id } => {
            let closed = commands::done(&SystemGit, &cwd, &id)?;
            if closed.already_done {
                println!("Issue {} was already done ({})", closed.id, closed.rel_path);
            } else {
                println!("Closed issue {} ({})", closed.id, closed.rel_path);
            }
        }
        Command::Grab { id } => {
            let grabbed = commands::grab(&SystemGit, &cwd, &id)?;
            println!(
                "Grabbed issue {} for {} ({})",
                grabbed.id, grabbed.assignee, grabbed.rel_path
            );
        }
    }

    Ok(())
}
