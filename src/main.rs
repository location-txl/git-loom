mod completions;
mod config;
mod copy;
mod doctor;
mod git;
mod worktree;

use clap::Parser;
use std::path::PathBuf;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "管理 Git 工作环境、关联仓库与本地配置",
    trailing_var_arg = true
)]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    dry_run: bool,
    /// status / branch / switch / checkout / diff / worktree / doctor / completions
    command: String,
    #[arg(allow_hyphen_values = true)]
    args: Vec<String>,
}
fn run(cli: Cli) -> Result<i32> {
    if cli.command == "completions" {
        return completions::run(&cli.args, cli.dry_run);
    }
    if cli.command == "doctor" {
        return doctor::run(cli.config.as_deref(), &cli.args);
    }
    if cli.command == "__complete" {
        if let Ok(ctx) = config::Context::load(cli.config.as_deref(), cli.repo.as_deref()) {
            let _ = completions::candidates(&ctx, &cli.args);
        }
        return Ok(0);
    }
    let ctx = config::Context::load(cli.config.as_deref(), cli.repo.as_deref())?;
    if cli.command == "worktree" {
        worktree::run(&ctx, &cli.args, cli.dry_run)
    } else {
        git::execute(&ctx, &cli.command, &cli.args, cli.dry_run)
    }
}
fn main() {
    let mut cli = Cli::parse();
    // clap consumes `--`; preserve the original Git argument boundary verbatim.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == "--config" || raw[index] == "--repo" {
            index += 2;
        } else if raw[index].starts_with('-') {
            index += 1;
        } else {
            cli.args = raw[index + 1..].to_vec();
            break;
        }
    }
    let code = match run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("错误：{error}");
            1
        }
    };
    std::process::exit(code);
}
