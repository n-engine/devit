//! RC1 Integration Tests - Patch Apply
//! Tests for atomic patch application with real file writes

mod common;
use common::*;
use serde_json::json;

fn mk_diff_replace(old: &str, newl: &str) -> String {
    format!(
        "\
--- src/hello.txt.orig\t2025-01-01 10:00:00 +0100
+++ src/hello.txt\t2025-01-01 10:01:00 +0100
@@ -1,3 +1,3 @@
 BEGIN
-{old}
+{newl}
 END
",
        old = old,
        newl = newl
    )
}

#[test]
fn patch_apply_atomic_ok() {
    let t = Tmp::new();

    // Initialize git repository
    assert!(run("git init", t.path()).status.success());
    assert!(run("git config user.email 'test@example.com'", t.path())
        .status
        .success());
    assert!(run("git config user.name 'Test User'", t.path())
        .status
        .success());

    t.write("src/hello.txt", "BEGIN\nOLD\nEND\n");

    // Initial commit
    assert!(run("git add .", t.path()).status.success());
    assert!(run("git commit -m 'Initial commit'", t.path())
        .status
        .success());

    // dry-run
    let diff = mk_diff_replace("OLD", "NEW");
    let json_input = json!({"patch": diff, "commit_dry_run": true}).to_string();
    t.write("input.json", &json_input);
    let cmd = format!(
        "devit fs-patch-apply --json {}/input.json",
        t.path().display()
    );
    let o = run(cmd, t.path());
    assert!(o.status.success(), "dry-run failed: {:?}", o);

    // apply
    let json_input = json!({"patch": diff}).to_string();
    t.write("input2.json", &json_input);
    let cmd = format!(
        "devit fs-patch-apply --json {}/input2.json",
        t.path().display()
    );
    let o = run(cmd, t.path());
    assert!(o.status.success(), "apply failed: {:?}", o);

    let after = t.read("src/hello.txt");
    assert!(after.contains("NEW"));
}

#[test]
fn patch_apply_idempotent() {
    let t = Tmp::new();
    t.write("src/hello.txt", "BEGIN\nOLD\nEND\n");

    let diff = mk_diff_replace("OLD", "NEW");
    let json_input = json!({"patch": diff}).to_string();
    let cmd = format!("echo '{}' | devit fs-patch-apply", json_input);
    assert!(run(&cmd, t.path()).status.success());
    // 2e apply → doit être no-op propre (sortie 0)
    assert!(run(&cmd, t.path()).status.success());

    let after = t.read("src/hello.txt");
    assert_eq!(after, "BEGIN\nNEW\nEND\n");
}

#[test]
fn patch_traversal_rejected() {
    let t = Tmp::new();
    let diff = "\
--- ../secrets.txt.orig\t2025-01-01 10:00:00 +0100
+++ ../secrets.txt\t2025-01-01 10:01:00 +0100
@@ -1,1 +1,1 @@
-secret
+oops
";
    let json_input = json!({"patch": diff}).to_string();
    let cmd = format!("echo '{}' | devit fs-patch-apply", json_input);
    let o = run(cmd, t.path());
    assert!(!o.status.success(), "traversal must fail");
}

#[test]
fn patch_fix_middleware_happy_path() {
    let t = Tmp::new();
    t.write("src/hello.txt", "BEGIN\nOLD\nEND\n");
    let pseudo = "\
*** Begin Patch
*** Update File: src/hello.txt
@@
- OLD
+ NEW
*** End Patch
";
    let cmd = format!(
        "devit mcp call devit_patch_fix \
         '{{\"diff\": {:?}, \"apply\": true, \"strict_mode\": false}}'",
        pseudo
    );
    let o = run(cmd, t.path());
    assert!(o.status.success(), "fixer path failed: {:?}", o);

    let after = t.read("src/hello.txt");
    assert!(after.contains("NEW"));
}
