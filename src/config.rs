use crate::{Result, copy, git};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub fn guide(args: &[String]) -> Result<i32> {
    if !args.is_empty() {
        return Err("Usage: git loom config".into());
    }
    print!("{}", include_str!("../docs/config.txt"));
    Ok(0)
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub copy_files: Vec<String>,
    #[serde(default)]
    pub peers: Vec<Peer>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Peer {
    pub path: PathBuf,
    #[serde(default)]
    pub branch_prefix: String,
    #[serde(default)]
    pub tag_prefix: String,
    #[serde(default)]
    pub copy_files: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct Repo {
    pub path: PathBuf,
    pub name: String,
    pub branch_prefix: String,
    pub tag_prefix: String,
    pub copy_files: Vec<String>,
}
pub struct Context {
    pub root: PathBuf,
    pub config_path: PathBuf,
    pub repos: Vec<Repo>,
    pub mapping: bool,
}
impl Context {
    pub fn load(config_path: Option<&Path>, selected: Option<&str>) -> Result<Self> {
        let explicit_config = config_path.is_some();
        let cwd = std::env::current_dir()?;
        let base = match config_path {
            Some(p) => cwd.join(p).parent().unwrap().to_path_buf(),
            None => cwd.clone(),
        };
        let root =
            PathBuf::from(git::output(&base, &["rev-parse", "--show-toplevel"])?).canonicalize()?;
        let config_path = config_path
            .map(|p| cwd.join(p))
            .unwrap_or_else(|| root.join(".git-loom.json"));
        let cfg: Config = if config_path.exists() {
            serde_json::from_slice(&std::fs::read(&config_path)?)?
        } else if explicit_config {
            return Err("The specified configuration file does not exist".into());
        } else {
            Config::default()
        };
        let mut repos = vec![Repo {
            path: root.clone(),
            name: ".".into(),
            branch_prefix: String::new(),
            tag_prefix: String::new(),
            copy_files: cfg.copy_files,
        }];
        for peer in cfg.peers {
            if peer.path.is_absolute() {
                return Err("peer.path must be a relative path".into());
            }
            let path = root
                .join(&peer.path)
                .canonicalize()
                .map_err(|e| format!("peer {}: {e}", peer.path.display()))?;
            let actual = PathBuf::from(git::output(&path, &["rev-parse", "--show-toplevel"])?)
                .canonicalize()?;
            if path != actual {
                return Err(
                    format!("peer must point to a repository root: {}", path.display()).into(),
                );
            }
            repos.push(Repo {
                path,
                name: peer.path.to_string_lossy().into_owned(),
                branch_prefix: peer.branch_prefix,
                tag_prefix: peer.tag_prefix,
                copy_files: peer.copy_files,
            });
        }
        let mut common_dirs = Vec::new();
        for (i, repo) in repos.iter().enumerate() {
            for prior in &repos[..i] {
                if repo.path.starts_with(&prior.path) || prior.path.starts_with(&repo.path) {
                    return Err("Member repository directories overlap or are nested".into());
                }
            }
            let common = git::common(&repo.path)?;
            if common_dirs.contains(&common) {
                return Err("The same Git repository cannot be linked more than once".into());
            }
            common_dirs.push(common);
            for prefix in [&repo.branch_prefix, &repo.tag_prefix] {
                git::output(
                    &repo.path,
                    &[
                        "check-ref-format",
                        &format!("refs/heads/{prefix}loom-check"),
                    ],
                )?;
            }
            copy::patterns(&repo.copy_files)?;
        }
        let mapping = selected.is_none() && repos.len() > 1;
        if let Some(selected) = selected {
            let target = root.join(selected).canonicalize()?;
            repos.retain(|r| r.path == target);
            if repos.is_empty() {
                return Err("--repo does not match a configured member".into());
            }
        }
        Ok(Self {
            root,
            config_path,
            repos,
            mapping,
        })
    }
}
