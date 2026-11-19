mod completions;
mod config;
mod copy;
mod doctor;
mod git;
mod worktree;

use clap::{Parser, Subcommand};
use std::{ffi::OsString, path::PathBuf};
const GIT_COMMANDS: &[(&str, &str)] = &[
    ("status", "Show working tree status"),
    ("branch", "List, create, or delete branches"),
    ("switch", "Switch branches"),
    ("checkout", "Switch branches or restore files"),
    ("diff", "Show changes"),
];
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser, Debug)]
#[command(
    bin_name = "git loom",
    version,
    about = "Manage Git work environments, linked repositories, and local configuration",
    arg_required_else_help = true,
    after_help = "Native Git commands: status, branch, switch, checkout, diff.\nPlace Loom options before COMMAND. Native Git arguments, including --, are passed after COMMAND."
)]
struct Cli {
    /// Read configuration from this file
    #[arg(long)]
    config: Option<PathBuf>,
    /// Select one repository and use its actual reference names
    #[arg(long)]
    repo: Option<String>,
    /// Preview supported operations without making changes
    #[arg(long)]
    dry_run: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show the configuration guide
    Config,
    /// Add, list, or remove work environments
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
    /// Check configuration and environment health
    Doctor {
        /// Print a versioned machine-readable report
        #[arg(long)]
        json: bool,
    },
    /// Print, install, or check shell completions
    Completions {
        #[command(subcommand)]
        command: CompletionCommand,
    },
    /// Remove the current executable; keep repository data and shell configuration
    Uninstall,
    #[command(name = "__complete", hide = true)]
    Complete { kind: String },
    #[command(external_subcommand)]
    Git(Vec<OsString>),
}

#[derive(Subcommand, Debug)]
enum WorktreeCommand {
    /// Create a work environment and copy its local configuration
    Add {
        directory: PathBuf,
        /// Create a new branch
        #[arg(short = 'b')]
        branch: Option<String>,
        start_point: Option<String>,
        /// Detach HEAD instead of checking out a branch
        #[arg(long, conflicts_with = "branch")]
        detach: bool,
    },
    /// List Loom environments and native Git worktrees
    List,
    /// Remove an environment, retaining its branches
    Remove {
        directory: PathBuf,
        /// Discard local changes in the selected environment
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
enum CompletionCommand {
    /// Generate Zsh completions with dynamic repository and reference candidates
    Zsh {
        /// Install completions and configure new interactive Zsh sessions
        #[arg(long)]
        install: bool,
        /// Check persistent completion configuration without modifying it
        #[arg(long, conflicts_with = "install")]
        status: bool,
        /// Override the completion directory
        #[arg(long, requires = "install")]
        dir: Option<PathBuf>,
    },
}

fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Commands::Config => config::guide(&[]),
        Commands::Completions {
            command:
                CompletionCommand::Zsh {
                    install,
                    dir,
                    status,
                },
        } => completions::run(install, dir, status, cli.dry_run),
        Commands::Doctor { json } => doctor::run(cli.config.as_deref(), json),
        Commands::Complete { kind } => {
            if let Ok(ctx) = config::Context::load(cli.config.as_deref(), cli.repo.as_deref()) {
                let _ = completions::candidates(&ctx, &[kind]);
            }
            Ok(0)
        }
        Commands::Uninstall => {
            let executable = std::env::current_exe()?;
            if cli.dry_run {
                println!("Would remove {}", executable.display());
            } else {
                std::fs::remove_file(&executable)?;
                println!("Uninstalled {}", executable.display());
            }
            Ok(0)
        }
        Commands::Worktree { command } => {
            let ctx = config::Context::load(cli.config.as_deref(), cli.repo.as_deref())?;
            match command {
                WorktreeCommand::Add {
                    directory,
                    branch,
                    start_point,
                    detach,
                } => worktree::add(
                    &ctx,
                    &directory,
                    branch.as_deref(),
                    start_point.as_deref(),
                    detach,
                    cli.dry_run,
                ),
                WorktreeCommand::List => worktree::list(&ctx),
                WorktreeCommand::Remove { directory, force } => {
                    worktree::remove(&ctx, &directory, force, cli.dry_run)
                }
            }
        }
        Commands::Git(args) => {
            let command = args
                .first()
                .and_then(|arg| arg.to_str())
                .ok_or("Missing Git command")?;
            if !GIT_COMMANDS.iter().any(|(name, _)| *name == command) {
                return Err(format!("Unknown command {command}; run git loom --help").into());
            }
            let ctx = config::Context::load(cli.config.as_deref(), cli.repo.as_deref())?;
            git::execute_os(&ctx, command, &args[1..], cli.dry_run)
        }
    }
}

fn main() {
    let code = match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    };
    std::process::exit(code);
}
