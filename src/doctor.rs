use crate::{Result, config::Context, copy, git, worktree};
use serde::Serialize;
use std::path::Path;
#[derive(Serialize)]
struct Check {
    id: String,
    level: String,
    repo: Option<String>,
    message: String,
    suggestion: String,
}
fn check(
    id: &str,
    level: &str,
    repo: Option<&Path>,
    message: impl Into<String>,
    suggestion: &str,
) -> Check {
    Check {
        id: id.into(),
        level: level.into(),
        repo: repo.map(|p| p.display().to_string()),
        message: message.into(),
        suggestion: suggestion.into(),
    }
}
pub fn run(path: Option<&Path>, args: &[String]) -> Result<i32> {
    if args.iter().any(|a| a != "--json") {
        return Err("doctor only supports --json".into());
    }
    let mut checks = Vec::new();
    match git::output(&std::env::current_dir()?, &["--version"]) {
        Ok(version) => checks.push(check("git", "OK", None, version, "")),
        Err(e) => checks.push(check(
            "git",
            "ERROR",
            None,
            e.to_string(),
            "Install Git and add it to PATH",
        )),
    }
    match Context::load(path, None) {
        Err(e) => checks.push(check(
            "configuration",
            "ERROR",
            None,
            e.to_string(),
            "Check JSON fields, repository paths, prefixes, and globs in the configuration selected by --config or the repository root",
        )),
        Ok(ctx) => {
            checks.push(check(
                "configuration",
                "OK",
                Some(&ctx.root),
                if ctx.config_path.exists() {
                    "Configuration and repository relationships are valid"
                } else {
                    "No configuration; using single-repository mode"
                },
                "",
            ));
            let primary_branch = git::output(&ctx.root, &["symbolic-ref", "--short", "HEAD"]).ok();
            for repo in &ctx.repos {
                checks.push(check(
                    "common-dir",
                    "OK",
                    Some(&repo.path),
                    git::common(&repo.path)?.display().to_string(),
                    "",
                ));
                match git::clean(&repo.path) {
                    Ok(()) => checks.push(check(
                        "working-tree",
                        "OK",
                        Some(&repo.path),
                        "Working tree is clean",
                        "",
                    )),
                    Err(e) => checks.push(check(
                        "working-tree",
                        "WARN",
                        Some(&repo.path),
                        e.to_string(),
                        "Commit or resolve local changes before switching branches",
                    )),
                }
                match git::output(&repo.path, &["symbolic-ref", "--short", "HEAD"]) {
                    Err(_) => checks.push(check(
                        "branch",
                        "WARN",
                        Some(&repo.path),
                        "HEAD is detached",
                        "Check whether you need to switch to a branch",
                    )),
                    Ok(branch) => {
                        let expected = primary_branch
                            .as_ref()
                            .map(|b| format!("{}{b}", repo.branch_prefix));
                        let mismatch = expected.as_ref().is_some_and(|b| b != &branch);
                        checks.push(check(
                            "branch",
                            if mismatch { "WARN" } else { "OK" },
                            Some(&repo.path),
                            format!(
                                "Current branch: {branch}; expected: {}",
                                expected.as_deref().unwrap_or("primary repository HEAD is detached")
                            ),
                            if mismatch {
                                "Check linked repositories against the primary repository branch"
                            } else {
                                ""
                            },
                        ));
                    }
                }
                match copy::scan(&repo.path, &repo.copy_files) {
                    Ok(matches) => {
                        checks.push(check(
                            "copy-files",
                            "OK",
                            Some(&repo.path),
                            format!("Matched {} files", matches.files.len()),
                            "",
                        ));
                        for warning in matches.warnings {
                            checks.push(check(
                                "copy-files",
                                "WARN",
                                Some(&repo.path),
                                warning,
                                "Check that copyFiles matches this project",
                            ));
                        }
                    }
                    Err(e) => checks.push(check(
                        "copy-files",
                        "ERROR",
                        Some(&repo.path),
                        e.to_string(),
                        "Check patterns and file permissions",
                    )),
                }
            }
            match worktree::groups(&ctx) {
                Err(e) => checks.push(check(
                    "worktrees",
                    "ERROR",
                    Some(&ctx.root),
                    e.to_string(),
                    "Check loom/worktrees.json in the Git common directory",
                )),
                Ok(groups) => {
                    for group in groups {
                        for member in group.members {
                            let registered = worktree::trees(&member.source)
                                .is_ok_and(|ts| ts.iter().any(|t| t.path == member.target));
                            let valid = member.created == registered
                                && (!member.created || member.target.is_dir());
                            checks.push(check(
                                "worktrees",
                                if valid && member.copied { "OK" } else { "WARN" },
                                Some(&member.target),
                                format!(
                                    "recorded={} registered={} configured={}",
                                    member.created, registered, member.copied
                                ),
                                if valid && member.copied {
                                    ""
                                } else {
                                    "Check records after partial creation or removal with native Git; doctor does not repair them automatically"
                                },
                            ));
                        }
                    }
                }
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        let installed = home.join(".zsh/completions/_git-loom").is_file();
        let rc = std::env::var_os("ZDOTDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or(home.clone())
            .join(".zshrc");
        let content = std::fs::read_to_string(rc).unwrap_or_default();
        let configured = content.lines().any(|l| {
            !l.trim_start().starts_with('#')
                && l.contains("fpath")
                && l.contains(".zsh/completions")
        });
        checks.push(check("zsh-completions", if installed && configured { "OK" } else { "WARN" }, None, format!("completion file in default directory={installed}; known .zshrc fpath configuration={configured} (static check; does not confirm the current shell has loaded it)"), "Run git loom completions zsh --install; in the current shell, run eval \"$(git loom completions zsh)\""));
    }
    let code = i32::from(checks.iter().any(|c| c.level == "ERROR"));
    if args.iter().any(|a| a == "--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"version": 1, "checks": checks}))?
        );
    } else {
        for item in checks {
            println!(
                "[{}] {} {}: {}",
                item.level,
                item.id,
                item.repo.as_deref().unwrap_or(""),
                item.message
            );
            if !item.suggestion.is_empty() {
                println!("  Suggestion: {}", item.suggestion);
            }
        }
    }
    Ok(code)
}
