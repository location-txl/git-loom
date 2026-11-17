use crate::{Result, config::Context, copy, git};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub source: PathBuf,
    pub target: PathBuf,
    pub created: bool,
    pub copied: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub target: PathBuf,
    pub members: Vec<Member>,
}
fn registry(ctx: &Context) -> Result<PathBuf> {
    Ok(git::common(&ctx.root)?.join("loom/worktrees.json"))
}
pub fn groups(ctx: &Context) -> Result<Vec<Group>> {
    let path = registry(ctx)?;
    if path.exists() {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    } else {
        Ok(Vec::new())
    }
}
fn save(ctx: &Context, groups: &[Group]) -> Result<()> {
    let path = registry(ctx)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(groups)?)?;
    fs::rename(temp, path)?;
    Ok(())
}
#[derive(Debug)]
pub struct Tree {
    pub path: PathBuf,
    pub locked: bool,
}
pub fn trees(repo: &Path) -> Result<Vec<Tree>> {
    let text = git::output(repo, &["worktree", "list", "--porcelain", "-z"])?;
    let mut trees: Vec<Tree> = Vec::new();
    for field in text.split('\0') {
        if let Some(path) = field.strip_prefix("worktree ") {
            trees.push(Tree {
                path: PathBuf::from(path),
                locked: false,
            });
        }
        if (field == "locked" || field.starts_with("locked "))
            && let Some(tree) = trees.last_mut()
        {
            tree.locked = true;
        }
    }
    Ok(trees)
}
fn absolute(path: &str) -> Result<PathBuf> {
    let raw = std::env::current_dir()?.join(path);
    let mut result = PathBuf::new();
    for component in raw.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            other => result.push(other),
        }
    }
    if result.exists() {
        Ok(result.canonicalize()?)
    } else {
        let mut ancestor = result.as_path();
        while !ancestor.exists() {
            ancestor = ancestor.parent().ok_or("目标没有可用父目录")?;
        }
        Ok(ancestor
            .canonicalize()?
            .join(result.strip_prefix(ancestor)?))
    }
}
pub fn run(ctx: &Context, args: &[String], dry: bool) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("add") => add(ctx, &args[1..], dry),
        Some("remove") => remove(ctx, &args[1..], dry),
        Some("list") => {
            for group in groups(ctx)? {
                println!("环境：{}", group.target.display());
                for member in group.members {
                    println!(
                        "  {} [创建={} 配置={}]",
                        member.target.display(),
                        member.created,
                        member.copied
                    );
                }
            }
            for repo in &ctx.repos {
                println!("仓库：{}（Git 实际登记）", repo.path.display());
                println!("{}", git::output(&repo.path, &["worktree", "list"])?);
            }
            Ok(0)
        }
        _ => Err(
            "用法：git loom worktree add <目录> [-b 分支] [起点] | list | remove <目录> [--force]"
                .into(),
        ),
    }
}
fn add(ctx: &Context, args: &[String], dry: bool) -> Result<i32> {
    let mut path = None;
    let mut branch = None;
    let mut start = None;
    let mut options = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-b" => {
                i += 1;
                branch = Some(args.get(i).ok_or("-b 缺少分支")?.clone());
            }
            "--detach" => options.push("--detach".to_string()),
            a if a.starts_with('-') => return Err(format!("worktree add 未支持参数 {a}").into()),
            a if path.is_none() => path = Some(a.to_string()),
            a if start.is_none() => start = Some(a.to_string()),
            _ => return Err("多余的 worktree 参数".into()),
        }
        i += 1;
    }
    let target = absolute(&path.ok_or("缺少目标目录")?)?;
    if target.exists() {
        return Err("目标目录已经存在，请使用新的环境目录".into());
    }
    let mut common = ctx.repos[0].path.clone();
    for repo in &ctx.repos {
        while !repo.path.starts_with(&common) {
            if !common.pop() {
                return Err("仓库不在同一文件系统根目录".into());
            }
        }
    }
    let mut records = groups(ctx)?;
    if records.iter().any(|g| g.target == target) {
        return Err("目标已有 Loom 创建记录，请先检查或删除该记录对应的环境".into());
    }
    let mut group = Group {
        target: target.clone(),
        members: Vec::new(),
    };
    let mut commands = Vec::new();
    let mut copies = Vec::new();
    for repo in &ctx.repos {
        let destination = if ctx.repos.len() == 1 {
            target.clone()
        } else {
            target.join(repo.path.strip_prefix(&common)?)
        };
        if destination.exists() {
            return Err(format!("目标已存在：{}", destination.display()).into());
        }
        let mut command = vec!["worktree".into(), "add".into()];
        command.extend(options.clone());
        let new = branch
            .as_deref()
            .map(|b| git::mapped(ctx, repo, b, true))
            .transpose()?;
        if let Some(new) = &new {
            git::branch_available(&repo.path, new, true)?;
            command.extend(["-b".into(), new.clone()]);
        }
        let reference = match start.as_deref() {
            Some(r) => git::mapped(ctx, repo, r, false)?,
            None if new.is_some() || !options.is_empty() => "HEAD".into(),
            None => return Err("请明确指定已有分支，或使用 -b 新分支".into()),
        };
        if !git::exists(&repo.path, &reference) {
            return Err(format!("{} 缺少引用 {reference}", repo.path.display()).into());
        }
        if new.is_none() && options.is_empty() {
            git::branch_available(&repo.path, &reference, false)?;
            if trees(&repo.path)?.iter().any(|t| {
                git::output(&t.path, &["symbolic-ref", "--short", "HEAD"])
                    .ok()
                    .as_deref()
                    == Some(
                        reference
                            .strip_prefix("refs/heads/")
                            .unwrap_or(reference.as_str()),
                    )
            }) {
                return Err(format!("分支 {reference} 已被检出").into());
            }
        }
        command.push(destination.to_string_lossy().into_owned());
        command.push(reference);
        let matched = copy::scan(&repo.path, &repo.copy_files)?;
        for warning in matched.warnings {
            eprintln!("{}：{warning}", repo.name);
        }
        eprintln!(
            "{} → {}，复制 {} 个文件",
            repo.path.display(),
            destination.display(),
            matched.files.len()
        );
        if dry {
            eprintln!("git {command:?}\n复制：{:?}", matched.files);
        }
        copies.push(matched.files);
        commands.push(command);
        group.members.push(Member {
            source: repo.path.clone(),
            target: destination,
            created: false,
            copied: false,
        });
    }
    if dry {
        return Ok(0);
    }
    let index = records.len();
    records.push(group);
    save(ctx, &records)?;
    for (i, command) in commands.iter().enumerate() {
        let member = &records[index].members[i];
        let status = Command::new("git")
            .arg("-C")
            .arg(&member.source)
            .args(command)
            .status()?;
        let registered = trees(&member.source)?
            .iter()
            .any(|tree| tree.path == member.target);
        records[index].members[i].created = registered;
        save(ctx, &records)?;
        if !status.success() {
            return Err(format!(
                "创建 {} 失败（Git 登记={}）；运行 git loom worktree list 查看，未自动回滚",
                records[index].members[i].target.display(),
                registered
            )
            .into());
        }
        let member = &records[index].members[i];
        copy::transfer(&member.source, &member.target, &copies[i]).map_err(|e| {
            format!(
                "worktree {} 已创建，但本地配置复制失败：{e}",
                member.target.display()
            )
        })?;
        if member.source == ctx.root && ctx.config_path.exists() {
            let config_target = member.target.join(".git-loom.json");
            if !config_target.exists() {
                fs::copy(&ctx.config_path, &config_target)?;
            } else if fs::read(&ctx.config_path)? != fs::read(&config_target)? {
                eprintln!("目标 .git-loom.json 与源配置不同，保留目标配置");
            }
        }
        records[index].members[i].copied = true;
        save(ctx, &records)?;
    }
    println!("开发环境已准备好：{}", target.display());
    Ok(0)
}
fn remove(ctx: &Context, args: &[String], dry: bool) -> Result<i32> {
    let force = args.iter().any(|a| a == "--force");
    let paths: Vec<_> = args.iter().filter(|a| a.as_str() != "--force").collect();
    if paths.len() != 1 {
        return Err("用法：worktree remove <目录> [--force]".into());
    }
    let target = absolute(paths[0])?;
    let mut records = groups(ctx)?;
    let index = records.iter().position(|g| g.target == target);
    let members = if let Some(i) = index {
        let selected = ctx
            .repos
            .iter()
            .map(|repo| git::common(&repo.path))
            .collect::<Result<Vec<_>>>()?;
        for member in &records[i].members {
            if !selected.contains(&git::common(&member.source)?) {
                return Err(
                    "环境包含未选中的仓库，拒绝整组删除；请去掉 --repo 并确认全部成员后重试".into(),
                );
            }
        }
        records[i].members.clone()
    } else if ctx.repos.len() == 1 {
        vec![Member {
            source: ctx.repos[0].path.clone(),
            target: target.clone(),
            created: true,
            copied: true,
        }]
    } else {
        return Err("多仓库删除只接受 Loom 登记的环境".into());
    };
    for member in members.iter().filter(|m| m.created) {
        let listing = trees(&member.source)?;
        let tree = listing
            .iter()
            .find(|t| t.path == member.target)
            .ok_or_else(|| {
                format!(
                    "{} 未登记为 worktree，请运行 doctor",
                    member.target.display()
                )
            })?;
        if listing.first().is_some_and(|t| t.path == tree.path) {
            return Err("不能删除主工作树".into());
        }
        if tree.locked {
            return Err(format!("{} 已锁定，请显式使用原生 Git 解锁", tree.path.display()).into());
        }
        if !force {
            git::clean(&member.target)?;
        }
    }
    if dry {
        for m in &members {
            eprintln!("删除 {}，force={force}", m.target.display());
        }
        return Ok(0);
    }
    for (i, member) in members.iter().enumerate().filter(|(_, m)| m.created) {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&member.source)
            .args(["worktree", "remove"]);
        if force {
            command.arg("--force");
        }
        let status = command.arg(&member.target).status()?;
        if !status.success() {
            return Err(format!(
                "删除 {} 失败，之前的删除已生效，未自动回滚",
                member.target.display()
            )
            .into());
        }
        if let Some(idx) = index {
            records[idx].members[i].created = false;
            save(ctx, &records)?;
        }
    }
    if let Some(index) = index {
        records.remove(index);
        save(ctx, &records)?;
    }
    if target.is_dir() && fs::read_dir(&target)?.next().is_none() {
        fs::remove_dir(&target)?;
    }
    println!("已删除 worktree，分支保留：{}", target.display());
    Ok(0)
}
