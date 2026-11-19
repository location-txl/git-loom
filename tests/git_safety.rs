use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    a: PathBuf,
    b: PathBuf,
}
fn git(path: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn loom(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-loom"))
        .current_dir(path)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap()
}
fn assert_ok(out: Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Match the canonical paths Loom reports, including macOS /private/var.
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a");
        let b = root.join("b");
        for repo in [&a, &b] {
            fs::create_dir(repo).unwrap();
            git(repo, &["init", "-b", "main"]);
            git(repo, &["config", "user.name", "Safety Test"]);
            git(repo, &["config", "user.email", "test@example.invalid"]);
            git(repo, &["commit", "--allow-empty", "-m", "initial"]);
        }
        fs::write(
            a.join(".git-loom.json"),
            r#"{"peers":[{"path":"../b","branchPrefix":"peer/"}]}"#,
        )
        .unwrap();
        git(&a, &["add", ".git-loom.json"]);
        git(&a, &["commit", "-m", "config"]);
        Self { _dir: dir, a, b }
    }
    fn branches(&self, name: &str) {
        git(&self.a, &["branch", name]);
        git(&self.b, &["branch", &format!("peer/{name}")]);
    }
    fn assert_branch(&self, repo: &Path, name: &str) {
        git(
            repo,
            &["rev-parse", "--verify", &format!("refs/heads/{name}")],
        );
    }
}

#[test]
fn deletion_missing_peer_preserves_root_and_all_requested_branches() {
    let f = Fixture::new();
    f.branches("one");
    git(&f.a, &["branch", "two"]);
    for args in [
        vec!["branch", "-d", "one", "two"],
        vec!["branch", "-D", "one", "two"],
    ] {
        assert!(!loom(&f.a, &args).status.success());
        f.assert_branch(&f.a, "one");
        f.assert_branch(&f.a, "two");
        f.assert_branch(&f.b, "peer/one");
    }
}

#[test]
fn deletion_checks_current_and_linked_worktree_even_when_forced() {
    let f = Fixture::new();
    f.branches("topic");
    git(&f.b, &["switch", "peer/topic"]);
    assert!(!loom(&f.a, &["branch", "-D", "topic"]).status.success());
    f.assert_branch(&f.a, "topic");
    git(&f.b, &["switch", "main"]);
    let linked = f._dir.path().join("linked");
    git(
        &f.b,
        &["worktree", "add", linked.to_str().unwrap(), "peer/topic"],
    );
    assert!(!loom(&f.a, &["branch", "-D", "topic"]).status.success());
    f.assert_branch(&f.a, "topic");
}

#[test]
fn deletion_uses_upstream_instead_of_head_and_force_is_explicit() {
    let f = Fixture::new();
    f.branches("topic");
    git(&f.b, &["switch", "peer/topic"]);
    git(&f.b, &["commit", "--allow-empty", "-m", "topic"]);
    git(&f.b, &["branch", "integrated"]);
    git(&f.b, &["switch", "main"]);
    // Native Git permits deletion merged into upstream even when not into HEAD.
    git(
        &f.b,
        &["branch", "--set-upstream-to=integrated", "peer/topic"],
    );
    assert_ok(loom(&f.a, &["branch", "-d", "topic"]));

    f.branches("second");
    git(&f.b, &["branch", "old-upstream"]);
    git(&f.b, &["commit", "--allow-empty", "-m", "head ahead"]);
    git(&f.b, &["branch", "-f", "peer/second", "HEAD"]);
    git(
        &f.b,
        &["branch", "--set-upstream-to=old-upstream", "peer/second"],
    );
    // HEAD includes the branch, but its upstream does not: -d must still reject.
    assert!(!loom(&f.a, &["branch", "-d", "second"]).status.success());
    f.assert_branch(&f.a, "second");
    assert_ok(loom(&f.a, &["branch", "--delete", "--force", "second"]));
}

#[test]
fn deletion_without_upstream_checks_head_and_dry_run_is_readonly() {
    let f = Fixture::new();
    f.branches("topic");
    git(&f.b, &["switch", "peer/topic"]);
    git(&f.b, &["commit", "--allow-empty", "-m", "topic"]);
    git(&f.b, &["switch", "main"]);
    assert!(!loom(&f.a, &["branch", "-d", "topic"]).status.success());
    f.assert_branch(&f.a, "topic");
    assert_ok(loom(&f.a, &["--dry-run", "branch", "-D", "topic"]));
    f.assert_branch(&f.a, "topic");
    f.assert_branch(&f.b, "peer/topic");
    assert_ok(loom(&f.a, &["branch", "-D", "--", "topic"]));
}

#[test]
fn ambiguous_and_remote_deletion_are_rejected_before_mutation() {
    let f = Fixture::new();
    f.branches("topic");
    for args in [
        vec!["branch", "-d", "-r", "topic"],
        vec!["branch", "-d", "--list", "topic"],
        vec!["branch", "-d", "-m", "topic"],
    ] {
        assert!(!loom(&f.a, &args).status.success());
        f.assert_branch(&f.a, "topic");
    }
}

#[test]
fn detached_checkout_can_read_branches_occupied_in_other_worktrees() {
    for command in ["switch", "checkout"] {
        let f = Fixture::new();
        f.branches("topic");
        for (repo, branch, suffix) in [
            (&f.a, "topic", "a-linked"),
            (&f.b, "peer/topic", "b-linked"),
        ] {
            let linked = f._dir.path().join(suffix);
            git(repo, &["worktree", "add", linked.to_str().unwrap(), branch]);
        }
        assert_ok(loom(&f.a, &[command, "--detach", "topic"]));
        assert_eq!(git(&f.a, &["rev-parse", "--abbrev-ref", "HEAD"]), "HEAD");
        assert_eq!(git(&f.b, &["rev-parse", "--abbrev-ref", "HEAD"]), "HEAD");
        // Selected-repo native arguments must work too.
        assert_ok(loom(&f.a, &["--repo", ".", command, "--detach", "topic"]));
        // Ordinary attachment still refuses the occupied branch.
        assert!(!loom(&f.a, &[command, "topic"]).status.success());
    }
}

#[test]
fn mapped_option_arity_is_scoped_to_each_git_command() {
    let f = Fixture::new();
    for args in [
        vec!["switch", "--output", "ignored", "main"],
        vec!["checkout", "--contains", "main"],
        vec!["diff", "--conflict", "merge"],
        vec!["status", "--sort", "refname"],
        vec!["branch", "--pathspec-from-file", "paths"],
    ] {
        let out = loom(&f.a, &args);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("Cannot reliably interpret option"));
    }
    assert_ok(loom(&f.a, &["diff", "-U2"]));
    assert_ok(loom(
        &f.a,
        &["branch", "--list", "--format=%(refname:short)"],
    ));
    assert_ok(loom(&f.a, &["status", "--short", "--branch"]));
}

#[test]
fn quiet_diff_aggregates_all_repositories_and_respects_path_boundary() {
    let f = Fixture::new();
    for repo in [&f.a, &f.b] {
        fs::write(repo.join("--quiet"), "original\n").unwrap();
        git(repo, &["add", "--", "--quiet"]);
        git(repo, &["commit", "-m", "file"]);
        fs::write(repo.join("--quiet"), "changed\n").unwrap();
    }
    let out = loom(&f.a, &["diff", "--quiet"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(&format!("== {} ==", f.a.display())));
    assert!(stderr.contains(&format!("== {} ==", f.b.display())));
    let path_only = loom(&f.a, &["diff", "--", "--quiet"]);
    assert_eq!(path_only.status.code(), Some(0));
    assert!(!path_only.stdout.is_empty());
}
