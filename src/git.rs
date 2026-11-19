use crate::{
    Result,
    config::{Context, Repo},
};
use std::{
    ffi::OsString,
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
pub(crate) fn mapped_branch(repo: &Repo, name: &str) -> String {
    if matches!(name, "main" | "master") {
        name.to_owned()
    } else {
        format!("{}{name}", repo.branch_prefix)
    }
}

pub fn mapped(ctx: &Context, repo: &Repo, value: &str, new_branch: bool) -> Result<String> {
    if !ctx.mapping {
        return Ok(value.to_string());
    }
    if new_branch {
        return Ok(mapped_branch(repo, value));
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
        return Ok(format!("refs/heads/{}{suffix}", mapped_branch(repo, name)));
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
        return Ok(format!("{}{suffix}", mapped_branch(repo, base)));
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
            "{full}{remote}/{}{suffix}",
            mapped_branch(repo, name)
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
        return Ok(format!("{}{suffix}", mapped_branch(repo, base)));
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

// A conservative, command-specific subset. Optional values must use =; options
// requiring the next argument are identified separately. Unknown mapped grammar
// is refused rather than interpreted as a revision or a pathname.
fn option_kind(command: &str, flag: &str) -> (bool, bool) {
    let known = match command {
        "switch" => matches!(
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
                | "--track"
                | "-t"
                | "--no-track"
        ),
        "checkout" => matches!(
            flag,
            "-q" | "--quiet"
                | "-f"
                | "--force"
                | "-m"
                | "--merge"
                | "--detach"
                | "-d"
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
        ),
        "branch" => matches!(
            flag,
            "-q" | "--quiet"
                | "-f"
                | "--force"
                | "-m"
                | "-M"
                | "--move"
                | "-c"
                | "-C"
                | "--copy"
                | "-d"
                | "-D"
                | "--delete"
                | "--list"
                | "-a"
                | "--all"
                | "-r"
                | "--remotes"
                | "-v"
                | "-vv"
                | "--show-current"
                | "--color"
                | "--no-color"
                | "--abbrev"
                | "--track"
                | "-t"
                | "--no-track"
                | "-l"
        ),
        "status" => matches!(
            flag,
            "--porcelain"
                | "--short"
                | "-s"
                | "--branch"
                | "-b"
                | "--untracked-files"
                | "--ignored"
        ),
        "diff" => matches!(
            flag,
            "-q" | "--quiet"
                | "-p"
                | "--patch"
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
                | "-M"
                | "-C"
                | "-c"
                | "-s"
        ),
        _ => false,
    };
    let takes_value = match command {
        "switch" => flag == "--conflict",
        "checkout" => matches!(flag, "--conflict" | "--pathspec-from-file"),
        "branch" => matches!(
            flag,
            "--format" | "--sort" | "--contains" | "--no-contains" | "--points-at"
        ),
        "diff" => matches!(
            flag,
            "--output"
                | "--diff-filter"
                | "--src-prefix"
                | "--dst-prefix"
                | "--line-prefix"
                | "--unified"
                | "-U"
                | "--word-diff-regex"
                | "--inter-hunk-context"
        ),
        _ => false,
    };
    (known, takes_value)
}

struct Arguments {
    args: Vec<String>,
    moving: bool,
    target: Option<String>,
    new: Option<String>,
    reset: bool,
    detached: bool,
    deletion: Vec<String>,
}
fn arguments(ctx: &Context, repo: &Repo, command: &str, args: &[String]) -> Result<Arguments> {
    let mut result = Arguments {
        args: Vec::new(),
        moving: matches!(command, "switch" | "checkout"),
        target: None,
        new: None,
        reset: false,
        detached: false,
        deletion: Vec::new(),
    };
    if command == "branch"
        && ctx.mapping
        && args
            .iter()
            .take_while(|a| a.as_str() != "--")
            .any(|a| matches!(a.as_str(), "-d" | "-D" | "--delete"))
    {
        return deletion_arguments(ctx, repo, args, result);
    }
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
            if matches!(command, "switch" | "checkout") && matches!(arg.as_str(), "--detach" | "-d")
            {
                result.detached = true;
                result.moving = true;
            }
            let flag = arg.split('=').next().unwrap();
            let (known, takes_value) = option_kind(command, flag);
            let attached_context = command == "diff"
                && arg.starts_with("-U")
                && arg[2..].bytes().all(|b| b.is_ascii_digit());
            if ctx.mapping && !(known || takes_value || attached_context) {
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
// Only this unambiguous subset is coordinated across repositories. Other branch
// modes still use their existing parser or explicit --repo native pass-through.
fn deletion_arguments(
    ctx: &Context,
    repo: &Repo,
    args: &[String],
    mut result: Arguments,
) -> Result<Arguments> {
    let mut after_separator = false;
    for arg in args {
        if !after_separator && arg == "--" {
            after_separator = true;
            result.args.push(arg.clone());
        } else if !after_separator && arg.starts_with('-') {
            match arg.as_str() {
                "-d" | "--delete" | "-q" | "--quiet" => {}
                "-D" | "-f" | "--force" => result.reset = true,
                _ => return Err(format!(
                    "Cannot safely coordinate branch deletion with {arg}; use --repo for native Git semantics"
                ).into()),
            }
            result.args.push(arg.clone());
        } else {
            // Git's previous-checkout shorthand is stateful, not a literal branch
            // name. Do not prepend a peer prefix or silently guess its meaning.
            if arg.contains("@{") || arg == "-" {
                return Err(
                    "Use literal branch names for coordinated deletion, or select --repo".into(),
                );
            }
            let name = mapped(ctx, repo, arg, true)?;
            output(&repo.path, &["check-ref-format", "--branch", &name])?;
            if result.deletion.contains(&name) {
                return Err(
                    format!("Branch {name} was requested for deletion more than once").into(),
                );
            }
            result.deletion.push(name.clone());
            result.args.push(name);
        }
    }
    if result.deletion.is_empty() {
        return Err("Branch deletion requires at least one branch name".into());
    }
    Ok(result)
}

fn deletion_available(repo: &Path, name: &str, force: bool) -> Result<()> {
    let reference = format!("refs/heads/{name}");
    if !exists(repo, &reference) {
        return Err(format!("{} is missing branch {name}", repo.display()).into());
    }
    // Unlike branch creation, deletion must also reject the current worktree.
    let listing = output(repo, &["worktree", "list", "--porcelain", "-z"])?;
    if listing
        .split('\0')
        .any(|field| field == format!("branch {reference}"))
    {
        return Err(format!(
            "Branch {name} in {} is checked out in a worktree",
            repo.display()
        )
        .into());
    }
    if !force {
        // Native `branch -d` checks its resolvable upstream, falling back to HEAD.
        // Checking only HEAD would either reject valid deletion or lose branches
        // not yet integrated into their configured upstream.
        let upstream = output(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{name}@{{upstream}}"),
            ],
        );
        let destination = upstream.unwrap_or_else(|_| "HEAD".to_string());
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["merge-base", "--is-ancestor", &reference, &destination])
            .output()?;
        match status.status.code() {
            Some(0) => {}
            Some(1) => return Err(format!("Branch {name} in {} is not fully merged into its upstream or HEAD; use -D only to intentionally discard it", repo.display()).into()),
            _ => return Err(format!("Cannot verify deletion of {name}: {}", String::from_utf8_lossy(&status.stderr).trim()).into()),
        }
    }
    Ok(())
}

pub fn execute_os(ctx: &Context, command: &str, args: &[OsString], dry: bool) -> Result<i32> {
    let mut after_separator = false;
    let mut parsed = Vec::with_capacity(args.len());
    for arg in args {
        if let Some(value) = arg.to_str() {
            after_separator |= value == "--";
            parsed.push(value.to_owned());
        } else if !ctx.mapping
            && after_separator
            && matches!(command, "checkout" | "diff" | "status")
        {
            // Only path operands after an explicit boundary are opaque. The
            // planner does not inspect them, and execution uses original bytes.
            parsed.push("<non-UTF-8 path>".to_owned());
        } else {
            return Err("Cannot safely interpret non-UTF-8 Git arguments here; use native Git, or put paths after -- in single-repository mode".into());
        }
    }
    execute_inner(ctx, command, &parsed, dry, (!ctx.mapping).then_some(args))
}

fn execute_inner(
    ctx: &Context,
    command: &str,
    args: &[String],
    dry: bool,
    native_args: Option<&[OsString]>,
) -> Result<i32> {
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
        for branch in &plan.deletion {
            deletion_available(&repo.path, branch, plan.reset)?;
        }
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
                && !plan.detached
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
            if let Some(args) = native_args {
                eprintln!("git {command} {args:?}");
            } else {
                eprintln!("git {command} {:?}", plan.args);
            }
            continue;
        }
        let mut process = Command::new("git");
        process.arg("-C").arg(&repo.path).arg(command);
        if let Some(args) = native_args {
            process.args(args);
        } else {
            process.args(&plan.args);
        }
        let status = process.status()?;
        let current = status.code().unwrap_or(1);
        let diff_found = command == "diff"
            && current == 1
            && args
                .iter()
                .take_while(|a| a.as_str() != "--")
                .any(|a| a == "--exit-code" || a == "--quiet");
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
