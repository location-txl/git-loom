use crate::{Result, config::Context, git, worktree};
use std::{
    env, fs, io,
    io::Write,
    path::{Path, PathBuf},
};
const SCRIPT: &str = include_str!("../completions/_git-loom");
const MARKER: &str = "# git-loom managed completion v1";
pub(crate) const START: &str = "# >>> git-loom zsh completions >>>";
const END: &str = "# <<< git-loom zsh completions <<<";
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
    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let rc_dir = env::var_os("ZDOTDIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.clone())
        .ok_or("Neither ZDOTDIR nor HOME is set; cannot locate .zshrc")?;
    let directory = match directory {
        Some(path) => path,
        None => home
            .ok_or("HOME is not set; use --dir")?
            .join(".zsh/completions"),
    };
    let target = std::path::absolute(directory.join("_git-loom"))?;
    let rc = std::path::absolute(rc_dir.join(".zshrc"))?;
    let target_spec = target
        .to_str()
        .filter(|s| !s.contains(['\n', '\r']))
        .ok_or("The completion path must be UTF-8 and cannot contain a newline")?;
    let old_completion = read_regular(&target)?;
    if old_completion
        .as_ref()
        .is_some_and(|(text, _)| !text.lines().any(|line| line == MARKER))
    {
        return Err(
            "The existing completion file is not managed by Loom; refusing to overwrite it".into(),
        );
    }
    let old_rc = read_regular(&rc)?;
    let startup = configure_rc(old_rc.as_ref().map_or("", |(text, _)| text), target_spec)?;
    if dry {
        println!("Would install: {}", target.display());
        println!("Would configure: {}", rc.display());
        return Ok(0);
    }

    write_if_changed(&target, &script, old_completion.as_ref())?;
    write_if_changed(&rc, &startup, old_rc.as_ref())?;
    println!("Installed: {}", target.display());
    println!("Configured: {}", rc.display());
    println!("Open a new Zsh terminal to use completions.");
    Ok(0)
}

fn read_regular(path: &Path) -> Result<Option<(String, fs::Permissions)>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {
            let text = fs::read_to_string(path)
                .map_err(|err| format!("Cannot read {}: {err}", path.display()))?;
            Ok(Some((text, meta.permissions())))
        }
        Ok(_) => Err(format!(
            "Refusing to modify a symlink or non-regular file: {}",
            path.display()
        )
        .into()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("Cannot inspect {}: {err}", path.display()).into()),
    }
}

fn configure_rc(old: &str, target: &str) -> Result<String> {
    let mut inside = false;
    let mut seen = false;
    let mut result = String::new();
    for line in old.split_inclusive('\n') {
        match line.trim_end_matches(['\r', '\n']) {
            START if !seen => {
                seen = true;
                inside = true;
            }
            END if inside => inside = false,
            START | END => return Err("Invalid Git Loom completion markers in .zshrc".into()),
            _ if !inside => result.push_str(line),
            _ => {}
        }
    }
    if inside {
        return Err("Unterminated Git Loom completion block in .zshrc".into());
    }
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    let target = shell_quote(target);
    result.push_str(&format!(
        "{START}\nif [[ -r {target} ]]; then\n  if (( ! $+functions[compdef] )); then\n    autoload -Uz compinit && compinit\n  fi\n  if (( $+functions[compdef] )); then\n    autoload -Uz -- {target}\n    compdef _git-loom git-loom\n  fi\nfi\n{END}\n"
    ));
    Ok(result)
}

fn write_if_changed(
    path: &Path,
    text: &str,
    old: Option<&(String, fs::Permissions)>,
) -> Result<()> {
    if old.is_some_and(|(contents, _)| contents == text) {
        return Ok(());
    }
    let write = || -> io::Result<()> {
        let parent = path
            .parent()
            .expect("installation paths are absolute files");
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(text.as_bytes())?;
        if let Some((_, permissions)) = old {
            file.as_file().set_permissions(permissions.clone())?;
            file.persist(path)?;
        } else {
            file.persist_noclobber(path)?;
        }
        Ok(())
    };
    write().map_err(|err| format!("Cannot write {}: {err}", path.display()).into())
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
