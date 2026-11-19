use crate::{Result, config::Context, git, worktree};
use std::{fs, path::PathBuf};
const SCRIPT: &str = include_str!("../completions/_git-loom");
const MARKER: &str = "# git-loom managed completion v1";
// Keep static completion vocabulary in sync with clap's actual command tree.
fn script() -> String {
    use clap::CommandFactory;
    let mut cli = crate::Cli::command();
    cli.build();
    fn flags(command: &clap::Command) -> String {
        command
            .get_arguments()
            .flat_map(|arg| {
                let short = arg.get_short().map(|s| format!("-{s}"));
                let long = arg.get_long().map(|s| format!("--{s}"));
                short.into_iter().chain(long)
            })
            .map(|flag| shell_quote(&flag))
            .collect::<Vec<_>>()
            .join(" ")
    }
    let mut commands: Vec<_> = cli
        .get_subcommands()
        .filter(|s| !s.is_hide_set())
        .map(|s| {
            shell_quote(&format!(
                "{}:{}",
                s.get_name(),
                s.get_about().map(ToString::to_string).unwrap_or_default()
            ))
        })
        .collect();
    commands.extend(
        crate::GIT_COMMANDS
            .iter()
            .map(|(name, about)| shell_quote(&format!("{name}:{about}"))),
    );
    let worktree = cli.find_subcommand("worktree").unwrap();
    let completion = cli.find_subcommand("completions").unwrap();
    let zsh = completion.find_subcommand("zsh").unwrap();
    let actions = worktree
        .get_subcommands()
        .map(|s| shell_quote(s.get_name()))
        .collect::<Vec<_>>()
        .join(" ");
    SCRIPT
        .replace("@LOOM_COMMANDS@", &commands.join(" "))
        .replace("@LOOM_FLAGS@", &flags(&cli))
        .replace(
            "@DOCTOR_FLAGS@",
            &flags(cli.find_subcommand("doctor").unwrap()),
        )
        .replace("@COMPLETION_FLAGS@", &flags(zsh))
        .replace("@WORKTREE_COMMANDS@", &actions)
        .replace(
            "@WORKTREE_ADD_FLAGS@",
            &flags(worktree.find_subcommand("add").unwrap()),
        )
        .replace(
            "@WORKTREE_REMOVE_FLAGS@",
            &flags(worktree.find_subcommand("remove").unwrap()),
        )
}

pub fn run(install: bool, directory: Option<PathBuf>, dry: bool) -> Result<i32> {
    let script = script();
    if !install {
        print!("{script}\n_git_loom_register\n");
        return Ok(0);
    }
    let directory = match directory {
        Some(p) => p,
        None => PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set; use --dir")?)
            .join(".zsh/completions"),
    };
    let target = directory.join("_git-loom");
    if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Refusing to overwrite a symlink completion file".into());
    }
    if target.exists()
        && !fs::read_to_string(&target)?
            .lines()
            .any(|line| line == MARKER)
    {
        return Err(
            "The existing completion file is not managed by Loom; refusing to overwrite it".into(),
        );
    }
    if dry {
        println!("Would install: {}", target.display());
        return Ok(0);
    }
    fs::create_dir_all(&directory)?;
    fs::write(&target, script)?;
    println!("Installed: {}", target.display());
    println!("Current shell: eval \"$(git loom completions zsh)\"");
    println!("In .zshrc, add this directory to fpath before compinit / Oh My Zsh initialization:");
    println!(
        "fpath=({} $fpath)",
        shell_quote(&directory.to_string_lossy())
    );
    println!("After the last compinit (after sourcing Oh My Zsh and any plugins):");
    println!("autoload -Uz _git-loom");
    println!("compdef _git-loom git-loom");
    println!("Open a new terminal and press Tab after 'git-loom doc' to verify.");
    Ok(0)
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn candidates(ctx: &Context, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("repos") => {
            for repo in &ctx.repos {
                println!("{}", repo.name);
            }
        }
        Some("worktrees") => {
            for group in worktree::groups(ctx)? {
                println!("{}", group.target.display());
            }
            if ctx.repos.len() == 1 {
                for tree in worktree::trees(&ctx.repos[0].path)?.into_iter().skip(1) {
                    println!("{}", tree.path.display());
                }
            }
        }
        _ => {
            let repo = if ctx.mapping {
                &ctx.root
            } else {
                &ctx.repos[0].path
            };
            println!(
                "{}",
                git::output(
                    repo,
                    &[
                        "for-each-ref",
                        "--format=%(refname:short)",
                        "refs/heads",
                        "refs/tags",
                        "refs/remotes"
                    ]
                )?
            );
        }
    }
    Ok(())
}
