//! RC1 Integration Tests - Test Runner
//! Tests for minimal test runner with cargo/shell detection

mod common;
use common::*;

fn mk_cargo_proj(t: &Tmp) {
    let cargo = "\
[package]
name = \"mini\"
version = \"0.1.0\"
edition = \"2021\"
[dev-dependencies]
";
    t.write("Cargo.toml", cargo);
    t.write("src/lib.rs", "pub fn add(a:i32,b:i32)->i32{a+b}\n");
    t.write("tests/t_ok.rs", "#[test] fn ok(){ assert_eq!(2+2,4);} ");
    t.write("tests/t_fail.rs", "#[test] fn ko(){ assert_eq!(2+2,5);} ");
}

#[test]
fn cargo_runner_reports_pass_fail() {
    let t = Tmp::new();
    mk_cargo_proj(&t);

    let o = run("devit test-run --json", t.path());
    assert!(o.status.success(), "runner failed");
    let out = String::from_utf8_lossy(&o.stdout);

    // attend un JSON avec counts (à implémenter côté runner)
    assert!(out.contains("\"passed\""));
    assert!(out.contains("\"failed\""));
}

#[test]
fn shell_runner_timeout() {
    let t = Tmp::new();
    t.write("tests.sh", "#!/bin/sh\nsleep 5\nexit 0\n");
    run("chmod +x tests.sh", t.path());

    let o = run(
        "devit test-run --shell ./tests.sh --timeout 1s --json",
        t.path(),
    );

    assert!(!o.status.success(), "timeout should fail");
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("\"timeout\":true"));
}
