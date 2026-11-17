use crate::{Result, config::Context, git, worktree};
use std::{fs, path::PathBuf};
const SCRIPT: &str = include_str!("../completions/_git-loom");
const MARKER: &str = "# git-loom managed completion v1";
pub fn run(args: &[String], dry: bool) -> Result<i32> {
    if args.first().map(String::as_str) != Some("zsh") {
        return Err("用法：completions zsh [--install] [--dir 路径]".into());
    }
    let mut install = false;
    let mut directory = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--install" => install = true,
            "--dir" => {
                i += 1;
                directory = Some(PathBuf::from(args.get(i).ok_or("--dir 缺少路径")?));
            }
            a => return Err(format!("未知补全参数 {a}").into()),
        }
        i += 1;
    }
    if !install {
        if directory.is_some() {
            return Err("--dir 需要 --install".into());
        }
        print!("{SCRIPT}\n_git_loom_register\n");
        return Ok(0);
    }
    let directory = match directory {
        Some(p) => p,
        None => PathBuf::from(std::env::var_os("HOME").ok_or("缺少 HOME，请使用 --dir")?)
            .join(".zsh/completions"),
    };
    let target = directory.join("_git-loom");
    if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("不覆盖符号链接补全文件".into());
    }
    if target.exists()
        && !fs::read_to_string(&target)?
            .lines()
            .any(|line| line == MARKER)
    {
        return Err("同名补全文件不属于 Loom，拒绝覆盖".into());
    }
    if dry {
        println!("将安装：{}", target.display());
        return Ok(0);
    }
    fs::create_dir_all(&directory)?;
    fs::write(&target, SCRIPT)?;
    println!("已安装：{}", target.display());
    println!("当前终端：eval \"$(git loom completions zsh)\"");
    println!(
        "持久加载：将该目录加入 fpath，放在 compinit / Oh My Zsh 初始化之前，然后开启新终端。"
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
