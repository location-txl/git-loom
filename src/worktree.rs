use crate::{Result, config::Context, copy, git};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
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
// Keep the lock on a stable inode: the JSON itself is atomically replaced.
fn lock_registry(ctx: &Context, dry: bool) -> Result<Option<fs::File>> {
    if dry {
        return Ok(None);
    }
    let path = registry(ctx)?.with_file_name("worktrees.lock");
    fs::create_dir_all(path.parent().ok_or("Invalid worktree registry path")?)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    match lock.try_lock() {
        Ok(()) => Ok(Some(lock)),
        Err(fs::TryLockError::WouldBlock) => Err(
            "Another git loom worktree operation is active for this repository; retry after it finishes".into(),
        ),
        Err(fs::TryLockError::Error(error)) => Err(error.into()),
    }
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
fn absolute(path: &Path) -> Result<PathBuf> {
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
            ancestor = ancestor
                .parent()
                .ok_or("Target has no usable parent directory")?;
        }
        Ok(ancestor
            .canonicalize()?
            .join(result.strip_prefix(ancestor)?))
    }
}
pub(crate) fn list(ctx: &Context) -> Result<i32> {
    for group in groups(ctx)? {
        println!("Environment: {}", group.target.display());
        for member in group.members {
            println!(
                "  {} [created={} configured={}]",
                member.target.display(),
                member.created,
                member.copied
            );
        }
    }
    for repo in &ctx.repos {
        println!("Repository: {} (registered with Git)", repo.path.display());
        println!("{}", git::output(&repo.path, &["worktree", "list"])?);
    }
    Ok(0)
}
pub(crate) fn add(
    ctx: &Context,
    path: &Path,
    branch: Option<&str>,
    start: Option<&str>,
    detach: bool,
    dry: bool,
) -> Result<i32> {
    let _registry_lock = lock_registry(ctx, dry)?;
    let target = absolute(path)?;
    if target.exists() {
        return Err("Target directory already exists; use a new environment directory".into());
    }
    let mut common = ctx.repos[0].path.clone();
    for repo in &ctx.repos {
        while !repo.path.starts_with(&common) {
            if !common.pop() {
                return Err("Repositories do not share a filesystem root".into());
            }
        }
    }
    let mut records = groups(ctx)?;
    if records.iter().any(|g| g.target == target) {
        return Err(
            "Target already has a Loom record; inspect or remove the recorded environment first"
                .into(),
        );
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
            return Err(format!("Target already exists: {}", destination.display()).into());
        }
        let mut command: Vec<OsString> = vec!["worktree".into(), "add".into()];
        if detach {
            command.push("--detach".into());
        }
        let new = branch
            .map(|b| git::mapped(ctx, repo, b, true))
            .transpose()?;
        if let Some(new) = &new {
            git::branch_available(&repo.path, new, true)?;
            command.extend(["-b".into(), new.into()]);
        }
        let reference = match start {
            Some(r) => git::mapped(ctx, repo, r, false)?,
            None if new.is_some() || detach => "HEAD".into(),
            None => {
                return Err("Specify an existing branch or use -b to create a new branch".into());
            }
        };
        if !git::exists(&repo.path, &reference) {
            return Err(format!("{} is missing reference {reference}", repo.path.display()).into());
        }
        if new.is_none() && !detach {
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
                return Err(format!("Branch {reference} is already checked out").into());
            }
        }
        command.push(destination.as_os_str().to_owned());
        command.push(reference.into());
        let matched = copy::scan(&repo.path, &repo.copy_files)?;
        for warning in matched.warnings {
            eprintln!("{}：{warning}", repo.name);
        }
        eprintln!(
            "{} -> {}, copying {} files",
            repo.path.display(),
            destination.display(),
            matched.files.len()
        );
        if dry {
            eprintln!("git {command:?}\nCopy: {:?}", matched.files);
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
                "Failed to create {} (registered with Git={}); run git loom worktree list to inspect; no automatic rollback",
                records[index].members[i].target.display(),
                registered
            )
            .into());
        }
        let member = &records[index].members[i];
        copy::transfer(&member.source, &member.target, &copies[i]).map_err(|e| {
            format!(
                "Worktree {} was created, but copying local configuration failed: {e}",
                member.target.display()
            )
        })?;
        if member.source == ctx.root && ctx.config_path.exists() {
            let config_target = member.target.join(".git-loom.json");
            if !config_target.exists() {
                fs::copy(&ctx.config_path, &config_target)?;
            } else if fs::read(&ctx.config_path)? != fs::read(&config_target)? {
                eprintln!(
                    "Destination .git-loom.json differs from the source; keeping the destination configuration"
                );
            }
        }
        records[index].members[i].copied = true;
        save(ctx, &records)?;
    }
    println!("Development environment is ready: {}", target.display());
    Ok(0)
}
pub(crate) fn remove(ctx: &Context, path: &Path, force: bool, dry: bool) -> Result<i32> {
    let _registry_lock = lock_registry(ctx, dry)?;
    let target = absolute(path)?;
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
                    "Environment contains unselected repositories; refusing group removal. Remove --repo and review all members before retrying".into(),
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
        return Err("Multi-repository removal requires a Loom-registered environment".into());
    };
    for member in members.iter().filter(|m| m.created) {
        let listing = trees(&member.source)?;
        let tree = listing
            .iter()
            .find(|t| t.path == member.target)
            .ok_or_else(|| {
                format!(
                    "{} is not registered as a worktree; run doctor",
                    member.target.display()
                )
            })?;
        if listing.first().is_some_and(|t| t.path == tree.path) {
            return Err("Cannot remove the main working tree".into());
        }
        if tree.locked {
            return Err(format!(
                "{} is locked; unlock it explicitly using native Git",
                tree.path.display()
            )
            .into());
        }
        if !force {
            git::clean(&member.target)?;
        }
    }
    if dry {
        for m in &members {
            eprintln!("Remove {}, force={force}", m.target.display());
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
                "Failed to remove {}; previous removals remain in effect; no automatic rollback",
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
    println!("Removed worktree; branch retained: {}", target.display());
    Ok(0)
}
