use std::{fs, path::Path, process::Command};

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn loom(repo: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_git-loom"));
    command.current_dir(repo);
    command
}

#[test]
fn concurrent_registry_mutation_is_rejected_and_lock_is_reusable() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    let registry = repo.join(".git/loom");
    fs::create_dir(&registry).unwrap();
    let lock = fs::File::create(registry.join("worktrees.lock")).unwrap();
    lock.lock().unwrap();

    let target = temp.path().join("new");
    for args in [&["add", "../new", "--detach"][..], &["remove", "../new"]] {
        let out = loom(&repo).arg("worktree").args(args).output().unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("operation is active"));
        assert!(!target.exists());
        assert!(!registry.join("worktrees.json").exists());
    }
    let dry = loom(&repo)
        .args(["--dry-run", "worktree", "add", "../new", "--detach"])
        .output()
        .unwrap();
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(!target.exists());

    drop(lock);
    let add = loom(&repo)
        .args(["worktree", "add", "../new", "--detach"])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    assert!(target.is_dir());
    let remove = loom(&repo)
        .args(["worktree", "remove", "../new"])
        .output()
        .unwrap();
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );
    assert!(!target.exists());
}
