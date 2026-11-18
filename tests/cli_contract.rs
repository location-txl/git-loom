use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn loom(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-loom"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn own_command_help_works_outside_repository() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec!["--help"],
        vec!["config", "--help"],
        vec!["doctor", "--help"],
        vec!["worktree", "--help"],
        vec!["worktree", "add", "--help"],
        vec!["worktree", "list", "--help"],
        vec!["worktree", "remove", "--help"],
        vec!["completions", "zsh", "--help"],
        vec!["uninstall", "--help"],
    ] {
        let out = loom(root.path(), &args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("Usage:"));
    }
}

#[test]
fn own_commands_reject_unknown_arguments_before_loading_repository() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec!["worktree", "list", "--typo"],
        vec!["doctor", "--typo"],
        vec!["config", "init"],
        vec!["completions", "zsh", "--dir", "anywhere"],
    ] {
        let out = loom(root.path(), &args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("Usage:"));
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn native_git_path_separator_survives_clap_and_global_options() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-b", "main"]);
    git(root.path(), &["config", "user.name", "test"]);
    git(
        root.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    fs::write(root.path().join("--strange"), "old\n").unwrap();
    git(root.path(), &["add", "--", "--strange"]);
    git(root.path(), &["commit", "-m", "initial"]);
    fs::write(root.path().join("--strange"), "new\n").unwrap();
    for args in [
        vec!["diff", "--name-only", "--", "--strange"],
        vec!["--repo=.", "diff", "--name-only", "--", "--strange"],
    ] {
        let out = loom(root.path(), &args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "--strange");
    }
}

#[test]
fn generated_completion_has_no_template_tokens_and_covers_own_commands() {
    let root = tempfile::tempdir().unwrap();
    let out = loom(root.path(), &["completions", "zsh"]);
    assert!(out.status.success());
    let script = String::from_utf8(out.stdout).unwrap();
    assert!(!script.contains("@LOOM_") && !script.contains("@WORKTREE_"));
    assert!(script.contains("'--json'") && script.contains("'--detach'"));
    assert!(script.contains("config:") && script.contains("switch:"));
}

// The macOS runner filesystem rejects this deliberately invalid UTF-8 filename.
#[test]
#[cfg(target_os = "linux")]
fn non_utf8_native_path_is_forwarded_without_loss() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-b", "main"]);
    git(root.path(), &["config", "user.name", "test"]);
    git(
        root.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    let name = OsString::from_vec(b"file-\xff".to_vec());
    fs::write(root.path().join(&name), "old\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "initial"]);
    fs::write(root.path().join(&name), "new\n").unwrap();
    let args = [
        OsString::from("diff"),
        OsString::from("--name-only"),
        OsString::from("-z"),
        OsString::from("--"),
        name,
    ];
    let expected = Command::new("git")
        .current_dir(root.path())
        .args(&args)
        .output()
        .unwrap();
    let actual = Command::new(env!("CARGO_BIN_EXE_git-loom"))
        .current_dir(root.path())
        .args(&args)
        .output()
        .unwrap();
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, expected.stdout);
    assert!(actual.stdout.contains(&0xff));
}
