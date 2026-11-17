use crate::{
    Result,
    config::{Context, Repo},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn output(path: &Path, args: &[&str]) -> Result<String> {
    let result = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr)
            .trim()
            .to_string()
            .into());
    }
    Ok(String::from_utf8(result.stdout)?
        .trim_end_matches('\n')
        .to_string())
}
pub fn common(path: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(output(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?)
    .canonicalize()?)
}
pub fn exists(path: &Path, reference: &str) -> bool {
    output(path, &["rev-parse", "--verify", "--quiet", reference]).is_ok()
}
pub fn clean(repo: &Path) -> Result<()> {
    let status = output(repo, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    if !status.is_empty() {
        return Err(format!("{} has local changes or files:\n{status}", repo.display()).into());
    }
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        let p = output(repo, &["rev-parse", "--git-path", marker])?;
        if repo.join(p).exists() {
            return Err(format!("{} has an unfinished Git operation", repo.display()).into());
        }
    }
    Ok(())
}
pub fn mapped(ctx: &Context, repo: &Repo, value: &str, new_branch: bool) -> Result<String> {
    if !ctx.mapping {
        return Ok(value.to_string());
    }
    if new_branch {
        return Ok(format!("{}{value}", repo.branch_prefix));
    }
    for separator in ["...", ".."] {
        if let Some((a, b)) = value.split_once(separator) {
            return Ok(format!(
                "{}{separator}{}",
                mapped(ctx, repo, a, false)?,
                mapped(ctx, repo, b, false)?
            ));
        }
    }
    let split = value.find(['~', '^', ':', '@']).unwrap_or(value.len());
    let (base, suffix) = value.split_at(split);
    if base.is_empty() || base == "HEAD" || base == "-" {
        return Ok(value.into());
    }
    if base.len() >= 4
        && base.bytes().all(|b| b.is_ascii_hexdigit())
        && exists(&ctx.root, base)
        && !exists(&ctx.root, &format!("refs/heads/{base}"))
        && !exists(&ctx.root, &format!("refs/tags/{base}"))
    {
        return Err("Cannot map commit SHAs across repositories; use --repo".into());
    }
    if let Some(name) = base.strip_prefix("refs/heads/") {
        return Ok(format!("refs/heads/{}{name}{suffix}", repo.branch_prefix));
    }
    if let Some(name) = base.strip_prefix("refs/tags/") {
        return Ok(format!("refs/tags/{}{name}{suffix}", repo.tag_prefix));
    }
    let branch = exists(&ctx.root, &format!("refs/heads/{base}"));
    let tag = exists(&ctx.root, &format!("refs/tags/{base}"));
    if branch && tag {
        return Err(format!(
            "Reference {base} is both a branch and a tag; use a fully qualified ref"
        )
        .into());
    }
    if branch {
        return Ok(format!("{}{base}{suffix}", repo.branch_prefix));
    }
    if tag {
        return Ok(format!("{}{base}{suffix}", repo.tag_prefix));
    }
    let remote_ref = base.strip_prefix("refs/remotes/").unwrap_or(base);
    if exists(&ctx.root, &format!("refs/remotes/{remote_ref}")) {
        let (remote, name) = remote_ref
            .split_once('/')
            .ok_or("Invalid remote reference")?;
        let full = if base.starts_with("refs/remotes/") {
            "refs/remotes/"
        } else {
            ""
        };
        return Ok(format!(
            "{full}{remote}/{}{name}{suffix}",
            repo.branch_prefix
        ));
    }
    // Preserve native checkout/switch remote branch inference.
    if output(
        &ctx.root,
        &["for-each-ref", "--format=%(refname)", "refs/remotes"],
    )?
    .lines()
    .any(|r| {
        r.strip_prefix("refs/remotes/")
            .and_then(|r| r.split_once('/'))
            .is_some_and(|(_, n)| n == base)
    }) {
        return Ok(format!("{}{base}{suffix}", repo.branch_prefix));
    }
    if exists(&ctx.root, base) {
        return Err(format!("Cannot map reference {base} across repositories; use --repo").into());
    }
    Ok(value.into())
}

pub fn branch_available(repo: &Path, name: &str, creating: bool) -> Result<()> {
    let name = name.strip_prefix("refs/heads/").unwrap_or(name);
    output(repo, &["check-ref-format", "--branch", name])?;
    if creating && exists(repo, &format!("refs/heads/{name}")) {
        return Err(format!("{} already has branch {name}", repo.display()).into());
    }
    let listing = output(repo, &["worktree", "list", "--porcelain", "-z"])?;
    let mut current = "";
    for line in listing.split('\0') {
        if let Some(p) = line.strip_prefix("worktree ") {
            current = p;
        }
        if line == format!("branch refs/heads/{name}") && Path::new(current) != repo {
            return Err(
                format!("Branch {name} is already checked out in worktree {current}").into(),
            );
        }
    }
    Ok(())
}

struct Arguments {
    args: Vec<String>,
    moving: bool,
    target: Option<String>,
    new: Option<String>,
    reset: bool,
}
fn arguments(ctx: &Context, repo: &Repo, command: &str, args: &[String]) -> Result<Arguments> {
    let mut result = Arguments {
        args: Vec::new(),
        moving: matches!(command, "switch" | "checkout"),
        target: None,
        new: None,
        reset: false,
    };
    let mut positionals = Vec::new();
    let mut after = false;
    let mut paths = false;
    let mut patch = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if after {
            result.args.push(arg.clone());
            i += 1;
            continue;
        }
        if arg == "--" {
            after = true;
            paths |= i + 1 < args.len();
            result.args.push(arg.clone());
            i += 1;
            continue;
        }
        if matches!(
            arg.as_str(),
            "-b" | "-B" | "-c" | "-C" | "--create" | "--force-create" | "--orphan"
        ) && matches!(command, "switch" | "checkout")
        {
            i += 1;
            let name = mapped(
                ctx,
                repo,
                args.get(i).ok_or("Missing new branch name")?,
                true,
            )?;
            result.new = Some(name.clone());
            result.reset = matches!(arg.as_str(), "-B" | "-C" | "--force-create");
            result.moving = true;
            result.args.extend([arg.clone(), name]);
        } else if arg.starts_with("--create=")
            || arg.starts_with("--force-create=")
            || arg.starts_with("--orphan=")
        {
            let (opt, name) = arg.split_once('=').unwrap();
            let name = mapped(ctx, repo, name, true)?;
            result.new = Some(name.clone());
            result.reset = opt == "--force-create";
            result.moving = true;
            result.args.push(format!("{opt}={name}"));
        } else if arg.starts_with('-') && arg != "-" {
            if matches!(arg.as_str(), "-p" | "--patch") {
                patch = true;
            }
            if arg == "--detach" || arg == "-d" && command == "switch" {
                result.moving = true;
            }
            let flag = arg.split('=').next().unwrap();
            let known = matches!(
                flag,
                "-q" | "--quiet"
                    | "-f"
                    | "--force"
                    | "-m"
                    | "--merge"
                    | "--detach"
                    | "-d"
                    | "--discard-changes"
                    | "--guess"
                    | "--no-guess"
                    | "--ignore-other-worktrees"
                    | "--recurse-submodules"
                    | "--no-recurse-submodules"
                    | "--progress"
                    | "--no-progress"
                    | "-p"
                    | "--patch"
                    | "--ours"
                    | "--theirs"
                    | "--overlay"
                    | "--no-overlay"
                    | "--ignore-skip-worktree-bits"
                    | "--pathspec-file-nul"
                    | "--track"
                    | "-t"
                    | "--no-track"
                    | "-l"
                    | "--cached"
                    | "--staged"
                    | "--stat"
                    | "--numstat"
                    | "--shortstat"
                    | "--name-only"
                    | "--name-status"
                    | "--raw"
                    | "--patch-with-stat"
                    | "--check"
                    | "--exit-code"
                    | "--color"
                    | "--no-color"
                    | "--word-diff"
                    | "--relative"
                    | "--binary"
                    | "--full-index"
                    | "--no-ext-diff"
                    | "--no-textconv"
                    | "--ignore-space-at-eol"
                    | "--ignore-all-space"
                    | "-w"
                    | "-b"
                    | "--minimal"
                    | "--patience"
                    | "--histogram"
                    | "--no-renames"
                    | "--find-renames"
                    | "--find-copies"
                    | "--abbrev"
                    | "--list"
                    | "-a"
                    | "--all"
                    | "-r"
                    | "--remotes"
                    | "-v"
                    | "-vv"
                    | "--show-current"
                    | "-M"
                    | "--move"
                    | "-c"
                    | "-C"
                    | "--copy"
                    | "-D"
                    | "--delete"
                    | "--porcelain"
                    | "--short"
                    | "-s"
                    | "--branch"
                    | "--untracked-files"
                    | "--ignored"
            );
            let takes_value = matches!(
                flag,
                "--conflict"
                    | "--pathspec-from-file"
                    | "--output"
                    | "--diff-filter"
                    | "--src-prefix"
                    | "--dst-prefix"
                    | "--line-prefix"
                    | "--unified"
                    | "-U"
                    | "--word-diff-regex"
                    | "--inter-hunk-context"
                    | "--format"
                    | "--sort"
                    | "--contains"
                    | "--no-contains"
                    | "--points-at"
            );
            if ctx.mapping && !known && !takes_value && !arg.starts_with("-U") {
                return Err(format!("Cannot reliably interpret option {arg} across repositories; use --repo to pass it through").into());
            }
            if flag == "--pathspec-from-file" {
                paths = true;
            }
            if matches!(flag, "--contains" | "--no-contains" | "--points-at") && arg.contains('=') {
                let (_, value) = arg.split_once('=').unwrap();
                result
                    .args
                    .push(format!("{flag}={}", mapped(ctx, repo, value, false)?));
            } else {
                result.args.push(arg.clone());
            }
            if takes_value && !arg.contains('=') {
                i += 1;
                let value = args.get(i).ok_or("Missing option argument")?;
                result.args.push(
                    if matches!(flag, "--contains" | "--no-contains" | "--points-at") {
                        mapped(ctx, repo, value, false)?
                    } else {
                        value.clone()
                    },
                );
            }
        } else {
            positionals.push((result.args.len(), arg.clone()));
            result.args.push(arg.clone());
        }
        i += 1;
    }
    if command == "checkout" && (patch || paths) && result.new.is_none() {
        result.moving = false;
    }
    if command == "checkout"
        && !patch
        && !paths
        && (!positionals.is_empty() || result.new.is_some())
    {
        result.moving = true;
    }
    if command == "branch"
        && args
            .iter()
            .any(|a| a == "--list" || a == "-a" || a == "-r" || a == "--all" || a == "--remotes")
    {
        return Ok(result);
    }
    for (n, (index, value)) in positionals.iter().enumerate() {
        let branch_edit = command == "branch"
            && args
                .iter()
                .any(|a| matches!(a.as_str(), "-m" | "-M" | "--move" | "-c" | "-C" | "--copy"));
        let branch_delete = command == "branch"
            && args
                .iter()
                .any(|a| matches!(a.as_str(), "-d" | "-D" | "--delete"));
        let creating = command == "branch"
            && !branch_delete
            && if branch_edit {
                n + 1 == positionals.len()
            } else {
                n == 0
            };
        if command == "branch"
            && args
                .iter()
                .any(|a| matches!(a.as_str(), "-f" | "--force" | "-M" | "-C"))
        {
            result.reset = true;
        }
        if matches!(command, "checkout" | "diff")
            && (if ctx.mapping { &ctx.root } else { &repo.path })
                .join(value)
                .exists()
            && !paths
        {
            if ctx.mapping {
                return Err(
                    format!("{value} may be a path; use -- to separate path arguments").into(),
                );
            }
            if command == "checkout" && !exists(&repo.path, value) {
                result.moving = false;
            }
        }
        if matches!(command, "status") {
            continue;
        }
        let mapped = mapped(ctx, repo, value, creating)?;
        if creating {
            result.new = Some(mapped.clone());
        } else if result.target.is_none() {
            result.target = Some(mapped.clone());
        }
        result.args[*index] = mapped;
    }
    Ok(result)
}
pub fn execute(ctx: &Context, command: &str, args: &[String], dry: bool) -> Result<i32> {
    if !matches!(
        command,
        "status" | "branch" | "switch" | "checkout" | "diff"
    ) {
        return Err(format!("Unknown command {command}").into());
    }
    let plans = ctx
        .repos
        .iter()
        .map(|repo| arguments(ctx, repo, command, args))
        .collect::<Result<Vec<_>>>()?;
    for (repo, plan) in ctx.repos.iter().zip(&plans) {
        if plan.moving {
            clean(&repo.path)?;
        }
        if let Some(name) = &plan.new {
            branch_available(&repo.path, name, !plan.reset)?;
        }
        if (plan.moving || command == "branch" && plan.new.is_some())
            && let Some(target) = &plan.target
        {
            if plan.moving
                && plan.new.is_none()
                && !args.iter().any(|a| a == "--ignore-other-worktrees")
                && exists(
                    &repo.path,
                    &format!(
                        "refs/heads/{}",
                        target.strip_prefix("refs/heads/").unwrap_or(target)
                    ),
                )
            {
                branch_available(&repo.path, target, false)?;
            }
            if !exists(&repo.path, target) && target != "-" {
                let refs = output(
                    &repo.path,
                    &["for-each-ref", "--format=%(refname)", "refs/remotes"],
                )?;
                if !refs.lines().any(|r| {
                    r.strip_prefix("refs/remotes/")
                        .and_then(|r| r.split_once('/'))
                        .is_some_and(|(_, n)| n == target)
                }) {
                    return Err(
                        format!("{} is missing reference {target}", repo.path.display()).into(),
                    );
                }
            }
        }
    }
    let mut code = 0;
    for (i, (repo, plan)) in ctx.repos.iter().zip(&plans).enumerate() {
        eprintln!("== {} ==", repo.path.display());
        if dry {
            eprintln!("git {command} {:?}", plan.args);
            continue;
        }
        let status = Command::new("git")
            .arg("-C")
            .arg(&repo.path)
            .arg(command)
            .args(&plan.args)
            .status()?;
        let current = status.code().unwrap_or(1);
        let diff_found = command == "diff"
            && current == 1
            && args.iter().any(|a| a == "--exit-code" || a == "--quiet");
        if current != 0 && !diff_found {
            eprintln!(
                "Completed: {}; failed: {}; not executed: {}. No automatic rollback; check git status in each repository before recovering.",
                ctx.repos[..i]
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                repo.name,
                ctx.repos[i + 1..]
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return Ok(current);
        }
        code = code.max(current);
    }
    Ok(code)
}
