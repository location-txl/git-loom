use crate::{Result, config::Context, git, worktree};
use std::{fs, path::PathBuf};
const SCRIPT: &str = include_str!("../completions/_git-loom");
const MARKER: &str = "# git-loom managed completion v1";
pub fn run(args: &[String], dry: bool) -> Result<i32> {
    if args.first().map(String::as_str) != Some("zsh") {
        return Err("Usage: completions zsh [--install] [--dir PATH]".into());
    }
    let mut install = false;
    let mut directory = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--install" => install = true,
            "--dir" => {
                i += 1;
                directory = Some(PathBuf::from(args.get(i).ok_or("--dir requires a path")?));
            }
            a => return Err(format!("Unknown completion argument {a}").into()),
        }
        i += 1;
    }
    if !install {
        if directory.is_some() {
            return Err("--dir requires --install".into());
        }
        print!("{SCRIPT}\n_git_loom_register\n");
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
    fs::write(&target, SCRIPT)?;
    println!("Installed: {}", target.display());
    println!("Current shell: eval \"$(git loom completions zsh)\"");
    println!(
        "To load persistently, add this directory to fpath before compinit / Oh My Zsh initialization, then open a new terminal."
    );
    println!(
        "fpath=({} $fpath)",
        shell_quote(&directory.to_string_lossy())
    );
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
