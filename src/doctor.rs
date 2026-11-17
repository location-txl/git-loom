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
        return Err("doctor 仅支持 --json".into());
    }
    let mut checks = Vec::new();
    match git::output(&std::env::current_dir()?, &["--version"]) {
        Ok(version) => checks.push(check("git", "OK", None, version, "")),
        Err(e) => checks.push(check(
            "git",
            "ERROR",
            None,
            e.to_string(),
            "安装 Git 并加入 PATH",
        )),
    }
    match Context::load(path, None) {
        Err(e) => checks.push(check(
            "configuration",
            "ERROR",
            None,
            e.to_string(),
            "检查 JSON 字段、仓库路径、前缀及 glob；配置位置以 --config 或仓库根目录为准",
        )),
        Ok(ctx) => {
            checks.push(check(
                "configuration",
                "OK",
                Some(&ctx.root),
                if ctx.config_path.exists() {
                    "配置与仓库关系有效"
                } else {
                    "无配置，按单仓库运行"
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
                        "工作区干净",
                        "",
                    )),
                    Err(e) => checks.push(check(
                        "working-tree",
                        "WARN",
                        Some(&repo.path),
                        e.to_string(),
                        "切换分支前提交或自行处理本地改动",
                    )),
                }
                match git::output(&repo.path, &["symbolic-ref", "--short", "HEAD"]) {
                    Err(_) => checks.push(check(
                        "branch",
                        "WARN",
                        Some(&repo.path),
                        "当前为 detached HEAD",
                        "检查是否需要切换到分支",
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
                                "当前分支：{branch}；预期：{}",
                                expected.as_deref().unwrap_or("主仓库 detached")
                            ),
                            if mismatch {
                                "按主仓库分支检查关联仓库"
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
                            format!("匹配 {} 个文件", matches.files.len()),
                            "",
                        ));
                        for warning in matches.warnings {
                            checks.push(check(
                                "copy-files",
                                "WARN",
                                Some(&repo.path),
                                warning,
                                "检查 copyFiles 是否符合当前项目",
                            ));
                        }
                    }
                    Err(e) => checks.push(check(
                        "copy-files",
                        "ERROR",
                        Some(&repo.path),
                        e.to_string(),
                        "检查模式与文件权限",
                    )),
                }
            }
            match worktree::groups(&ctx) {
                Err(e) => checks.push(check(
                    "worktrees",
                    "ERROR",
                    Some(&ctx.root),
                    e.to_string(),
                    "检查 Git 公共目录内 loom/worktrees.json",
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
                                    "登记={} 实际={} 配置完成={}",
                                    member.created, registered, member.copied
                                ),
                                if valid && member.copied {
                                    ""
                                } else {
                                    "检查部分创建或原生 Git 删除后的记录；doctor 不自动修复"
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
        checks.push(check("zsh-completions", if installed && configured { "OK" } else { "WARN" }, None, format!("默认目录补全文件存在={installed}；已知 .zshrc fpath 配置={configured}（静态检查，不代表当前 Shell 已加载）"), "运行 git loom completions zsh --install；当前终端执行 eval \"$(git loom completions zsh)\""));
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
                "[{}] {} {}：{}",
                item.level,
                item.id,
                item.repo.as_deref().unwrap_or(""),
                item.message
            );
            if !item.suggestion.is_empty() {
                println!("  建议：{}", item.suggestion);
            }
        }
    }
    Ok(code)
}
