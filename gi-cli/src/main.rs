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
#[command(name = "gi", version, about = "A decentralized, git-native issue tracker")]
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
    }

    Ok(())
}
