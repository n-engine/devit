//! RC1 Integration Tests - Snapshots
//! Tests for persistent snapshots with create/restore roundtrip

mod common;
use common::*;

#[test]
fn snapshot_create_restore_roundtrip() {
    let t = Tmp::new();
    t.write("src/a.txt", "A1\n");
    t.write("src/b.txt", "B1\n");

    assert!(run("devit snapshot create --name s1", t.path())
        .status
        .success());

    // Patch change
    let diff = "\
--- src/a.txt.orig\t2025-01-01 10:00:00 +0100
+++ src/a.txt\t2025-01-01 10:01:00 +0100
@@ -1,1 +1,1 @@
-A1
+A2
";
    assert!(run(
        &format!("devit patch-apply <<'EOF'\n{}\nEOF", diff),
        t.path()
    )
    .status
    .success());

    // Restore
    assert!(run("devit snapshot restore --name s1", t.path())
        .status
        .success());

    assert_eq!(t.read("src/a.txt"), "A1\n");
    assert_eq!(t.read("src/b.txt"), "B1\n");
}

#[test]
fn snapshot_retention_lru() {
    let t = Tmp::new();
    t.write("src/a.txt", "A\n");
    // set retention 2
    assert!(run("devit snapshot config --max 2", t.path())
        .status
        .success());

    for i in 1..=3 {
        assert!(
            run(&format!("devit snapshot create --name s{}", i), t.path())
                .status
                .success()
        );
    }

    // La s1 doit avoir été purgée
    let o = run("devit snapshot list", t.path());
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("s2") && s.contains("s3") && !s.contains("s1"));
}
