//! RC1 Integration Tests - Journal HMAC
//! Tests for HMAC-signed journal with real writes and verification

mod common;
use common::*;
use std::path::PathBuf;

#[test]
fn journal_written_and_verifiable() {
    let t = Tmp::new();
    t.write("src/x.txt", "X\n");

    // clé HMAC pour le run (fixe pour test)
    std::env::set_var("DEVIT_JOURNAL_KEY", "testkey123");

    // Run simple: snapshot + patch
    assert!(run("devit snapshot create --name j1", t.path())
        .status
        .success());

    let diff = "\
--- src/x.txt.orig\t2025-01-01 10:00:00 +0100
+++ src/x.txt\t2025-01-01 10:01:00 +0100
@@ -1,1 +1,1 @@
-X
+Y
";
    assert!(run(
        &format!("devit patch-apply <<'EOF'\n{}\nEOF", diff),
        t.path()
    )
    .status
    .success());

    // trouve le journal (par défaut .devit/journal.jsonl)
    let journal = t.path().join(".devit/journal.jsonl");
    assert!(journal.exists(), "journal absent");

    assert!(jverify(&journal, "testkey123"), "jverify failed");
}

#[test]
fn journal_break_detection() {
    let t = Tmp::new();
    t.write("src/k.txt", "K\n");
    std::env::set_var("DEVIT_JOURNAL_KEY", "key987");

    assert!(run("devit snapshot create --name jx", t.path())
        .status
        .success());

    let journal = t.path().join(".devit/journal.jsonl");
    assert!(journal.exists());

    // corrompre une ligne
    let mut s = t.read(".devit/journal.jsonl");
    s.push_str("{BROKEN}\n");
    std::fs::write(&journal, s).unwrap();

    assert!(!jverify(&journal, "key987"), "expected verify failure");
}
