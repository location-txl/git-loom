use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

#[test]
fn uninstall_removes_only_the_running_executable() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("git-loom");
    fs::copy(env!("CARGO_BIN_EXE_git-loom"), &executable).unwrap();
    let config = dir.path().join(".git-loom.json");
    fs::write(&config, "invalid config, must not be read or removed").unwrap();
    for args in [vec!["uninstall", "--help"], vec!["--dry-run", "uninstall"]] {
        let output = Command::new(&executable)
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(executable.exists());
    }
    let output = Command::new(&executable)
        .current_dir(dir.path())
        .args(["uninstall", "unexpected"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(executable.exists());
    let output = Command::new(&executable)
        .current_dir(dir.path())
        .arg("uninstall")
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(!executable.exists());
    assert!(config.exists());
}

fn git(path: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn init(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "-b", "main"]);
    git(path, &["config", "user.name", "Loom Test"]);
    git(path, &["config", "user.email", "loom@example.invalid"]);
    fs::write(path.join("file.txt"), "initial\n").unwrap();
    fs::write(path.join(".gitignore"), ".env*\nlocal.properties\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-m", "initial"]);
}
fn loom(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-loom"))
        .current_dir(path)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap()
}
fn ok(path: &Path, args: &[&str]) -> Output {
    let o = loom(path, args);
    assert!(
        o.status.success(),
        "loom {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    o
}
fn fail(path: &Path, args: &[&str]) -> Output {
    let o = loom(path, args);
    assert!(!o.status.success(), "unexpected success {args:?}");
    o
}

#[test]
fn config_guide_works_without_loading_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let output = ok(dir.path(), &["config"]);
    let guide = String::from_utf8(output.stdout).unwrap();
    assert!(guide.contains(".git-loom.json"));
    assert!(guide.contains("branchPrefix"));
    assert!(guide.contains("git loom doctor"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    fs::write(dir.path().join(".git-loom.json"), "invalid JSON").unwrap();
    ok(dir.path(), &["config"]);
    let output = fail(dir.path(), &["config", "init"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: git loom config"));
    assert_eq!(
        fs::read_to_string(dir.path().join(".git-loom.json")).unwrap(),
        "invalid JSON"
    );
}

struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    a: PathBuf,
    b: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("robot food");
        let b = root.join("platform");
        init(&a);
        init(&b);
        fs::write(a.join(".git-loom.json"), r#"{"copyFiles":["local.properties",".env",".env.*"],"peers":[{"path":"../platform","branchPrefix":"tob/","tagPrefix":"tob/","copyFiles":[".env"]}]}"#).unwrap();
        git(&a, &["add", ".git-loom.json"]);
        git(&a, &["commit", "-m", "config"]);
        Self {
            _dir: dir,
            root,
            a,
            b,
        }
    }
    fn target(&self) -> PathBuf {
        self.root.join("新环境 space")
    }
}
#[test]
fn switches_all_and_refuses_every_dirty_kind() {
    let f = Fixture::new();
    ok(&f.a, &["branch", "fix/中文"]);
    for kind in 0..3 {
        let file = if kind == 0 { "new.txt" } else { "file.txt" };
        fs::write(f.b.join(file), "dirty").unwrap();
        if kind == 2 {
            git(&f.b, &["add", file]);
        }
        fail(&f.a, &["checkout", "-f", "fix/中文"]);
        assert_eq!(git(&f.a, &["branch", "--show-current"]), "main");
        if kind == 0 {
            fs::remove_file(f.b.join(file)).unwrap();
        } else {
            git(&f.b, &["reset", "--hard", "HEAD"]);
        }
    }
    ok(&f.a, &["switch", "fix/中文"]);
    assert_eq!(git(&f.b, &["branch", "--show-current"]), "tob/fix/中文");
}
#[test]
fn worktree_copy_and_strict_remove() {
    let f = Fixture::new();
    fs::write(
        f.a.join("local.properties"),
        "project.cookplatform.dir=../platform\n",
    )
    .unwrap();
    fs::write(f.a.join(".env.local"), "TOKEN=not-printed").unwrap();
    fs::write(f.b.join(".env"), "OTHER=value").unwrap();
    fs::write(f.a.join("file.txt"), "dirty-source").unwrap();
    let target = f.target();
    let t = target.to_str().unwrap();
    ok(&f.a, &["worktree", "add", t, "-b", "fix/work"]);
    let a = target.join("robot food");
    let b = target.join("platform");
    assert_eq!(fs::read_to_string(a.join("file.txt")).unwrap(), "initial\n");
    assert_eq!(
        fs::read(a.join("local.properties")).unwrap(),
        fs::read(f.a.join("local.properties")).unwrap()
    );
    assert!(b.join(".env").is_file());
    assert_eq!(git(&b, &["branch", "--show-current"]), "tob/fix/work");
    ok(&a, &["status"]);
    fs::write(target.join("keep.txt"), "keep").unwrap();
    ok(&f.a, &["worktree", "remove", t]);
    assert!(target.join("keep.txt").exists());
    assert!(!a.exists() && !b.exists());
    assert!(f.a.join(".env.local").exists());
    assert!(f.b.join(".env").exists());
    assert!(git(&f.a, &["branch", "--list", "fix/work"]).contains("fix/work"));
}

#[test]
fn worktree_remove_refuses_tracked_changes_and_nonignored_untracked_files() {
    for kind in ["unstaged", "staged", "untracked"] {
        let f = Fixture::new();
        fs::write(f.a.join(".env"), "local config").unwrap();
        let target = f.target();
        let t = target.to_str().unwrap();
        ok(&f.a, &["worktree", "add", t, "-b", "remove-check"]);
        let peer = target.join("platform");
        let filename = if kind == "untracked" {
            "new.txt"
        } else {
            "file.txt"
        };
        fs::write(peer.join(filename), "precious changes").unwrap();
        if kind == "staged" {
            git(&peer, &["add", filename]);
        }
        fail(&f.a, &["worktree", "remove", t]);
        assert!(target.join("robot food/.git").exists());
        assert_eq!(
            fs::read_to_string(target.join("robot food/.env")).unwrap(),
            "local config"
        );
        assert_eq!(
            fs::read_to_string(peer.join(filename)).unwrap(),
            "precious changes"
        );
    }
}
#[test]
fn dry_run_and_missing_peer_branch_do_not_mutate() {
    let f = Fixture::new();
    git(&f.a, &["branch", "only-a"]);
    fail(&f.a, &["switch", "only-a"]);
    assert_eq!(git(&f.a, &["branch", "--show-current"]), "main");
    ok(&f.a, &["--dry-run", "switch", "-c", "new"]);
    assert!(git(&f.a, &["branch", "--list", "new"]).is_empty());
    ok(
        &f.a,
        &[
            "--dry-run",
            "worktree",
            "add",
            f.target().to_str().unwrap(),
            "-b",
            "new",
        ],
    );
    assert!(!f.target().exists());
}
#[test]
fn tags_sha_and_relative_refs() {
    let f = Fixture::new();
    git(&f.a, &["tag", "v1"]);
    git(&f.b, &["tag", "tob/v1"]);
    ok(&f.a, &["diff", "main...v1"]);
    let sha = git(&f.a, &["rev-parse", "HEAD"]);
    fail(&f.a, &["diff", &sha]);
    ok(&f.a, &["--repo", ".", "diff", &sha]);
    ok(&f.a, &["checkout", "--detach", "v1"]);
    assert!(git(&f.b, &["branch", "--show-current"]).is_empty());
}
#[test]
fn checkout_restores_files_and_diff_exit_code_aggregates() {
    let f = Fixture::new();
    fs::write(f.a.join("file.txt"), "changed\n").unwrap();
    let output = loom(&f.a, &["diff", "--exit-code"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("platform"));
    ok(&f.a, &["checkout", "--", "file.txt"]);
    assert_eq!(
        fs::read_to_string(f.a.join("file.txt")).unwrap(),
        "initial\n"
    );
}
#[test]
fn single_repo_works_without_config_and_from_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    init(&root);
    fs::create_dir(root.join("sub")).unwrap();
    ok(&root.join("sub"), &["switch", "-c", "feature"]);
    let target = dir.path().join("target");
    ok(
        &root,
        &["worktree", "add", target.to_str().unwrap(), "-b", "new"],
    );
    assert!(target.join("file.txt").is_file());
    assert!(!target.join("repo").exists());
    ok(&root, &["worktree", "remove", target.to_str().unwrap()]);
}
#[test]
fn doctor_json_reports_invalid_config_and_is_readonly() {
    let f = Fixture::new();
    for invalid in [
        "{",
        r#"{"copyFile":[]}"#,
        r#"{"copyFiles":5}"#,
        r#"{"copyFiles":["../secret"]}"#,
        r#"{"copyFiles":["["]}"#,
        r#"{"peers":[{"path":"../missing"}]}"#,
    ] {
        fs::write(f.a.join(".git-loom.json"), invalid).unwrap();
        let before = git(&f.a, &["status", "--porcelain"]);
        let out = fail(&f.a, &["doctor", "--json"]);
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["version"], 1);
        assert!(
            json["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["level"] == "ERROR")
        );
        assert_eq!(git(&f.a, &["status", "--porcelain"]), before);
    }
}
#[test]
fn glob_copies_nested_hidden_files_and_skips_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    init(&root);
    fs::write(
        root.join(".git-loom.json"),
        r#"{"copyFiles":["apps/**/.env.local",".env*",".env.local"]}"#,
    )
    .unwrap();
    git(&root, &["add", ".git-loom.json"]);
    git(&root, &["commit", "-m", "config"]);
    fs::create_dir_all(root.join("apps/demo/nested")).unwrap();
    fs::write(root.join("apps/demo/nested/.env.local"), "nested").unwrap();
    fs::write(root.join(".env.local"), "local").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(".env.local", root.join(".env.link")).unwrap();
    let target = dir.path().join("target");
    ok(
        &root,
        &["worktree", "add", target.to_str().unwrap(), "-b", "new"],
    );
    assert_eq!(
        fs::read_to_string(target.join("apps/demo/nested/.env.local")).unwrap(),
        "nested"
    );
    assert!(!target.join(".env.link").exists());
}
fn install_completion(home: &Path, zdotdir: Option<&Path>, extra_args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_git-loom"));
    cmd.current_dir(home)
        .env("HOME", home)
        .env_remove("ZDOTDIR");
    if let Some(dir) = zdotdir {
        cmd.env("ZDOTDIR", dir);
    }
    cmd.args(extra_args).output().unwrap()
}

#[test]
fn completion_install_and_dynamic_candidates() {
    let f = Fixture::new();
    let directory = f.root.join("completions");
    let args = [
        "completions",
        "zsh",
        "--install",
        "--dir",
        directory.to_str().unwrap(),
    ];
    assert!(install_completion(&f.root, None, &args).status.success());
    let rc = f.root.join(".zshrc");
    let first = fs::read_to_string(&rc).unwrap();
    assert!(install_completion(&f.root, None, &args).status.success());
    assert_eq!(fs::read_to_string(&rc).unwrap(), first);
    fs::write(directory.join("_git-loom"), "external").unwrap();
    assert!(!install_completion(&f.root, None, &args).status.success());
    assert_eq!(fs::read_to_string(rc).unwrap(), first);
    assert_eq!(
        fs::read_to_string(directory.join("_git-loom")).unwrap(),
        "external"
    );
    let o = ok(&f.a, &["__complete", "repos"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("../platform"));
    fs::write(f.a.join(".git-loom.json"), "bad json").unwrap();
    let o = ok(&f.a, &["__complete", "refs"]);
    assert!(o.stdout.is_empty() && o.stderr.is_empty());
    let script = ok(&f.root, &["completions", "zsh"]);
    let script_path = f.root.join("completion.zsh");
    fs::write(&script_path, script.stdout).unwrap();
    let out = Command::new("zsh")
        .arg("-n")
        .arg(&script_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn three_repositories_and_linked_peer_source() {
    let f = Fixture::new();
    let original = f.root.join("original-platform");
    fs::rename(&f.b, &original).unwrap();
    git(&original, &["branch", "-m", "other"]);
    git(
        &original,
        &["worktree", "add", "-b", "main", f.b.to_str().unwrap()],
    );
    let c = f.root.join("third");
    init(&c);
    fs::write(f.a.join(".git-loom.json"), r#"{"peers":[{"path":"../platform","branchPrefix":"tob/"},{"path":"../third","branchPrefix":"third/"}]}"#).unwrap();
    git(&f.a, &["add", ".git-loom.json"]);
    git(&f.a, &["commit", "-m", "third"]);
    ok(&f.a, &["checkout", "-b", "feature"]);
    assert_eq!(git(&c, &["branch", "--show-current"]), "third/feature");
    ok(
        &f.a,
        &["worktree", "add", f.target().to_str().unwrap(), "-b", "new"],
    );
    assert!(f.target().join("third/file.txt").exists());
    assert!(f.target().join("platform/.git").is_file());
}
#[test]
fn locked_worktree_blocks_entire_remove() {
    let f = Fixture::new();
    let target = f.target();
    let t = target.to_str().unwrap();
    ok(&f.a, &["worktree", "add", t, "-b", "new"]);
    git(
        &f.b,
        &[
            "worktree",
            "lock",
            target.join("platform").to_str().unwrap(),
        ],
    );
    fail(&f.a, &["worktree", "remove", t, "--force"]);
    assert!(target.join("robot food/.git").exists());
}
#[test]
fn orphan_checkout_and_existing_destination_file() {
    let f = Fixture::new();
    ok(&f.a, &["checkout", "--orphan", "empty"]);
    assert_eq!(git(&f.b, &["symbolic-ref", "--short", "HEAD"]), "tob/empty");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    init(&root);
    fs::write(root.join(".git-loom.json"), r#"{"copyFiles":["file.txt"]}"#).unwrap();
    git(&root, &["add", ".git-loom.json"]);
    git(&root, &["commit", "-m", "config"]);
    fs::write(root.join("file.txt"), "local change").unwrap();
    let target = dir.path().join("target");
    ok(
        &root,
        &["worktree", "add", target.to_str().unwrap(), "-b", "new"],
    );
    assert_eq!(
        fs::read_to_string(target.join("file.txt")).unwrap(),
        "initial\n"
    );
}
#[test]
fn checkout_patch_runs_with_dirty_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    init(&root);
    fs::write(root.join("file.txt"), "changed\n").unwrap();
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_git-loom"))
        .current_dir(&root)
        .args(["checkout", "-p", "--", "file.txt"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"y\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.join("file.txt")).unwrap(),
        "initial\n"
    );
}
#[test]
fn zsh_eval_and_autoload_dispatch_dynamic_completions() {
    let f = Fixture::new();
    git(&f.b, &["branch", "tob/topic"]);
    let bin = Path::new(env!("CARGO_BIN_EXE_git-loom")).parent().unwrap();
    let script = r#"
      autoload -Uz compinit
      compinit -D -i
      eval "$(git-loom completions zsh)"
      _files() { return 0; }
      _describe() { return 0; }
      compadd() { if [[ $1 == -a ]]; then print -l -- "${(@P)2}"; else print -l -- "$@"; fi }
      words=(loom switch '')
      CURRENT=3
      _git-loom
      words=(git-loom --repo ../platform switch '')
      CURRENT=5
      _git-loom
      words=(git loom doctor '')
      CURRENT=4
      _git-loom
    "#;
    let out = Command::new("zsh")
        .args(["-f", "-c", script])
        .current_dir(&f.a)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let output = String::from_utf8_lossy(&out.stdout);
    assert!(output.lines().any(|l| l == "main"), "{output}");
    assert!(output.lines().any(|l| l == "tob/topic"), "{output}");
    assert!(output.contains("--json"), "{output}");
    let directory = f.root.join("zsh");
    let out = install_completion(
        &f.root,
        None,
        &[
            "completions",
            "zsh",
            "--install",
            "--dir",
            directory.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("zsh").args(["-f", "-c", "fpath=($1 $fpath); autoload -Uz _git-loom; _describe() { return 0; }; _files() { return 0; }; compadd() { print -l -- $@; }; words=(loom doctor ''); CURRENT=3; _git-loom", "zsh", directory.to_str().unwrap()]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("--json"));
}

#[test]
fn zsh_installer_registers_after_a_stale_compinit_dump() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().canonicalize().unwrap();
    let zdotdir = home.join("dot files");
    let directory = "completion dir's";
    fs::create_dir(&zdotdir).unwrap();
    let rc = zdotdir.join(".zshrc");
    let original = r#"
autoload -Uz compinit
compinit -i -d "$ZDOTDIR/stale"
compinit -C -d "$ZDOTDIR/stale"
print -r -- "${_comps[git-loom]:-missing}"
"#;
    fs::write(&rc, original).unwrap();
    fs::set_permissions(&rc, fs::Permissions::from_mode(0o640)).unwrap();
    let args = ["completions", "zsh", "--install", "--dir", directory];
    let install = install_completion(&home, Some(&zdotdir), &args);
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    assert!(!home.join(".zshrc").exists());
    assert!(home.join(directory).join("_git-loom").is_file());
    let configured = fs::read_to_string(&rc).unwrap();
    assert!(configured.starts_with(original));
    assert_eq!(
        fs::metadata(&rc).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        install_completion(&home, Some(&zdotdir), &args)
            .status
            .success()
    );
    assert_eq!(fs::read_to_string(&rc).unwrap(), configured);

    let script = r#"
print -r -- "${_comps[git-loom]:-missing}"
words=(git-loom doctor '')
CURRENT=3
compadd() { print -l -- "$@"; }
"${_comps[git-loom]}"
"#;
    let out = Command::new("zsh")
        .args(["-ic", script])
        .current_dir(&zdotdir)
        .env("HOME", &home)
        .env("ZDOTDIR", &zdotdir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("missing\n_git-loom\n"), "{stdout}");
    assert!(stdout.contains("--json"), "{stdout}");
}

#[test]
fn zsh_installer_supports_a_fresh_home_and_refuses_unmanaged_startup_targets() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let rc = home.join(".zshrc");
    let target = home.join(".zsh/completions/_git-loom");
    let actual = home.join("dotfile");
    let args = ["completions", "zsh", "--install"];
    fs::write(&actual, "# user configuration").unwrap();
    symlink(&actual, &rc).unwrap();
    assert!(!install_completion(home, None, &args).status.success());
    assert!(fs::symlink_metadata(&rc).unwrap().is_symlink());
    assert_eq!(fs::read_to_string(&actual).unwrap(), "# user configuration");
    assert!(!target.exists());
    fs::remove_file(&rc).unwrap();
    fs::write(&rc, "# >>> git-loom zsh completions >>>\n").unwrap();
    assert!(!install_completion(home, None, &args).status.success());
    assert!(!target.exists());
    fs::remove_file(&rc).unwrap();
    assert!(
        install_completion(
            home,
            None,
            &["--dry-run", "completions", "zsh", "--install"]
        )
        .status
        .success()
    );
    assert!(!rc.exists() && !target.exists());
    let out = install_completion(home, None, &args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("zsh")
        .args(["-ic", "print -r -- ${_comps[git-loom]:-missing}"])
        .env("HOME", home)
        .env_remove("ZDOTDIR")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "_git-loom");
}

#[test]
fn zsh_completes_attached_repo_values_and_direct_native_options() {
    let f = Fixture::new();
    let bin = env!("CARGO_BIN_EXE_git-loom");
    let script = r#"
      autoload -Uz compinit
      compinit -D -i
      eval "$("$1" completions zsh)"
      _files() { return 0; }
      compadd() { return 0; }
      _describe() {
        if [[ $1 == repositories ]]; then local -a choices=("${(@P)2}"); local choice; for choice in "$choices[@]"; do print -r -- "$IPREFIX|$choice"; done; fi
        return 0
      }
      compset() {
        [[ $1 == -P && $2 == --repo= && $PREFIX == --repo=* ]] || return 1
        PREFIX=${PREFIX#--repo=}
        IPREFIX=--repo=
      }
      _git() {
        [[ $service == git && $CURRENT == 3 && $words[1] == git &&
           $words[2] == switch && $words[3] == --detac ]] || return 1
        print -r -- native:--detach
      }

      words=(git loom '--repo=../pl')
      CURRENT=3
      PREFIX=--repo=../pl
      IPREFIX=''
      service=git
      _git-loom

      words=(git-loom switch --detac)
      CURRENT=3
      PREFIX=--detac
      IPREFIX=''
      service=git-loom
      _git-loom
    "#;
    let out = Command::new("zsh")
        .args(["-f", "-c", script, "zsh", bin])
        .current_dir(&f.a)
        .env(
            "PATH",
            format!(
                "{}:{}",
                Path::new(bin).parent().unwrap().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let output = String::from_utf8_lossy(&out.stdout);
    assert!(
        output.lines().any(|l| l == "--repo=|../platform"),
        "{output}"
    );
    assert!(output.lines().any(|l| l == "native:--detach"), "{output}");
}

#[test]
#[cfg(unix)]
fn failed_post_checkout_records_created_worktree_and_can_remove() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let hook = f.b.join(".git/hooks/post-checkout");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    fail(
        &f.a,
        &["worktree", "add", f.target().to_str().unwrap(), "-b", "new"],
    );
    assert!(f.target().join("platform/.git").exists());
    let out = ok(&f.a, &["doctor", "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "worktrees" && c["level"] == "WARN")
    );
    ok(
        &f.a,
        &[
            "worktree",
            "remove",
            f.target().to_str().unwrap(),
            "--force",
        ],
    );
    assert!(!f.target().exists());
}
#[test]
fn primary_branches_are_literal_across_repositories() {
    let f = Fixture::new();
    ok(&f.a, &["branch", "master"]);
    ok(&f.a, &["switch", "master"]);
    assert_eq!(git(&f.b, &["branch", "--show-current"]), "master");

    let out = ok(&f.a, &["doctor", "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let peer = f.b.to_str().unwrap();
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "branch" && c["repo"] == peer && c["level"] == "OK")
    );

    ok(&f.a, &["switch", "main"]);
    assert_eq!(git(&f.b, &["branch", "--show-current"]), "main");
    ok(&f.a, &["branch", "-d", "master"]);
    assert!(git(&f.b, &["branch", "--list", "master", "tob/master"]).is_empty());
    ok(&f.a, &["branch", "fix/main"]);
    assert!(!git(&f.b, &["branch", "--list", "tob/fix/main"]).is_empty());

    let target = f.target();
    ok(
        &f.a,
        &[
            "worktree",
            "add",
            target.to_str().unwrap(),
            "-b",
            "master",
            "main",
        ],
    );
    assert_eq!(
        git(&target.join("platform"), &["branch", "--show-current"]),
        "master"
    );
}

#[test]
fn primary_branch_refs_preserve_qualifiers_and_do_not_exempt_tags() {
    let f = Fixture::new();
    for repo in [&f.a, &f.b] {
        git(repo, &["commit", "--allow-empty", "-m", "second"]);
        git(repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    }
    git(&f.a, &["tag", "main"]);
    git(&f.b, &["tag", "tob/main"]);
    ok(
        &f.a,
        &["diff", "refs/heads/main~1..refs/remotes/origin/main", "--"],
    );
    ok(&f.a, &["diff", "origin/main", "refs/tags/main", "--"]);
    let out = fail(&f.a, &["diff", "main", "--"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("both a branch and a tag"));

    // A branch found only via a remote-tracking ref should still use Git's
    // normal branch inference for the unprefixed name.
    for repo in [&f.a, &f.b] {
        git(repo, &["remote", "add", "origin", "."]);
        git(repo, &["update-ref", "refs/remotes/origin/master", "HEAD"]);
    }
    ok(&f.a, &["switch", "master"]);
    assert_eq!(git(&f.b, &["branch", "--show-current"]), "master");
}

#[test]
fn rename_and_force_create_keep_branch_prefixes() {
    let f = Fixture::new();
    ok(&f.a, &["branch", "-f", "feature"]);
    assert!(git(&f.b, &["branch", "--list", "tob/feature"]).contains("tob/feature"));
    ok(&f.a, &["branch", "-m", "feature", "renamed"]);
    assert!(git(&f.b, &["branch", "--list", "tob/renamed"]).contains("tob/renamed"));
}

#[test]
#[cfg(unix)]
fn copy_failure_preserves_created_status_and_stops_peers() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::write(f.a.join(".env"), "private").unwrap();
    let hook = f.a.join(".git/hooks/post-checkout");
    // Git creates the worktree successfully, then the hook removes the source file.
    let mut contents = b"#!/bin/sh\nrm -- ".to_vec();
    let quoted = format!(
        "'{}'\n",
        f.a.join(".env").to_string_lossy().replace('\'', "'\\''")
    );
    contents.extend(quoted.as_bytes());
    fs::write(&hook, contents).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let o = fail(
        &f.a,
        &["worktree", "add", f.target().to_str().unwrap(), "-b", "new"],
    );
    assert!(String::from_utf8_lossy(&o.stderr).contains("copying local configuration failed"));
    assert!(f.target().join("robot food/.git").exists());
    assert!(!f.target().join("platform").exists());
    ok(
        &f.a,
        &[
            "worktree",
            "remove",
            f.target().to_str().unwrap(),
            "--force",
        ],
    );
}
#[test]
fn explicit_missing_config_is_error_and_completion_dry_run_is_readonly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    init(&root);
    fail(&root, &["--config", ".git-loom.json", "doctor", "--json"]);
    let target = dir.path().join("completions");
    ok(
        &root,
        &[
            "--dry-run",
            "completions",
            "zsh",
            "--install",
            "--dir",
            target.to_str().unwrap(),
        ],
    );
    assert!(!target.exists());
}

#[test]
fn branch_creation_preflights_start_but_allows_occupied_base() {
    let f = Fixture::new();
    git(&f.a, &["branch", "missing-peer"]);
    fail(&f.a, &["branch", "new", "missing-peer"]);
    assert!(git(&f.a, &["branch", "--list", "new"]).is_empty());
    ok(
        &f.a,
        &[
            "worktree",
            "add",
            f.target().to_str().unwrap(),
            "-b",
            "occupied",
        ],
    );
    ok(&f.a, &["switch", "-c", "from-occupied", "occupied"]);
    assert_eq!(
        git(&f.b, &["branch", "--show-current"]),
        "tob/from-occupied"
    );
}

#[test]
fn empty_path_separator_does_not_bypass_switch_protection() {
    let f = Fixture::new();
    ok(&f.a, &["branch", "feature"]);
    fs::write(f.b.join("file.txt"), "precious changes").unwrap();
    fail(&f.a, &["checkout", "-f", "feature", "--"]);
    assert_eq!(git(&f.a, &["branch", "--show-current"]), "main");
    assert_eq!(git(&f.b, &["branch", "--show-current"]), "main");
    assert_eq!(
        fs::read_to_string(f.b.join("file.txt")).unwrap(),
        "precious changes"
    );
    fail(
        &f.a,
        &[
            "--repo",
            "../platform",
            "checkout",
            "-f",
            "tob/feature",
            "--",
        ],
    );
    assert_eq!(
        fs::read_to_string(f.b.join("file.txt")).unwrap(),
        "precious changes"
    );
}

#[test]
fn selected_repo_cannot_remove_unselected_group_members() {
    let f = Fixture::new();
    let target = f.target();
    let t = target.to_str().unwrap();
    ok(&f.a, &["worktree", "add", t, "-b", "group"]);
    let precious = target.join("robot food/.env");
    fs::write(&precious, "precious config").unwrap();
    fail(
        &f.a,
        &["--repo", "../platform", "worktree", "remove", t, "--force"],
    );
    assert_eq!(fs::read_to_string(&precious).unwrap(), "precious config");
    assert!(target.join("platform/.git").exists());
    ok(&f.a, &["worktree", "remove", t, "--force"]);
    assert!(!target.exists());

    ok(&f.a, &["--repo", ".", "worktree", "add", t, "-b", "single"]);
    fail(
        &f.a,
        &["--repo", "../platform", "worktree", "remove", t, "--force"],
    );
    assert!(target.join(".git").exists());
    ok(&f.a, &["--repo", ".", "worktree", "remove", t, "--force"]);
    assert!(!target.exists());
}
