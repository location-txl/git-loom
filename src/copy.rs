use crate::Result;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
use walkdir::WalkDir;

pub fn patterns(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if pattern.is_empty()
            || Path::new(pattern).is_absolute()
            || pattern.split('/').any(|p| p == ".." || p == ".git")
            || pattern.contains('\\')
        {
            return Err(format!(
                "非法复制模式：{pattern}；必须是仓库内的相对路径，不能包含 .git 或 .."
            )
            .into());
        }
        builder.add(GlobBuilder::new(pattern).literal_separator(true).build()?);
    }
    Ok(builder.build()?)
}
pub struct Matches {
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}
pub fn scan(root: &Path, pats: &[String]) -> Result<Matches> {
    let set = patterns(pats)?;
    let mut result = Matches {
        files: Vec::new(),
        warnings: Vec::new(),
    };
    if pats.is_empty() {
        return Ok(result);
    }
    if set.is_match(".git") || set.is_match(".git/config") || set.is_match(".git/objects/example") {
        result
            .warnings
            .push("模式覆盖 Git 元数据；.git 始终排除".into());
    }
    let mut counts = vec![0; pats.len()];
    let max_depth = if pats.iter().any(|p| p.contains("**")) {
        usize::MAX
    } else {
        pats.iter().map(|p| p.split('/').count()).max().unwrap_or(1)
    };
    for entry in WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || e.file_name() != ".git")
    {
        let entry = entry?;
        let relative = entry.path().strip_prefix(root)?;
        let matches = set.matches(relative);
        if matches.is_empty() {
            continue;
        }
        if entry.file_type().is_symlink() {
            result
                .warnings
                .push(format!("跳过符号链接 {}", relative.display()));
        } else if entry.file_type().is_file() {
            for i in matches {
                counts[i] += 1;
            }
            result.files.push(relative.to_path_buf());
        }
    }
    for (pattern, count) in pats.iter().zip(counts) {
        if count == 0 {
            result.warnings.push(format!("没有匹配文件：{pattern}"));
        }
    }
    result.files.sort();
    Ok(result)
}
pub fn transfer(source: &Path, target: &Path, files: &[PathBuf]) -> Result<()> {
    for relative in files {
        let src = source.join(relative);
        let dst = target.join(relative);
        if fs::symlink_metadata(&dst).is_ok() {
            eprintln!("保留已有文件：{}", dst.display());
            continue;
        }
        let mut parent = target.to_path_buf();
        for part in relative.parent().unwrap_or(Path::new("")).components() {
            if !matches!(part, Component::Normal(_)) {
                return Err("复制路径越界".into());
            }
            parent.push(part);
            if let Ok(meta) = fs::symlink_metadata(&parent) {
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    return Err(format!("目标父路径不是普通目录：{}", parent.display()).into());
                }
            } else {
                fs::create_dir(&parent)?;
            }
        }
        let meta = fs::symlink_metadata(&src)?;
        if !meta.is_file() {
            return Err(format!("源文件已变化：{}", src.display()).into());
        }
        let mut input = fs::File::open(&src)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dst)?;
        fs::set_permissions(&dst, meta.permissions())?;
        std::io::copy(&mut input, &mut output)?;
        eprintln!("已复制：{}", dst.display());
    }
    Ok(())
}
