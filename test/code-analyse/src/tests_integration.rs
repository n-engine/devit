use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;
use std::fs;

#[test]
fn test_analyze_bad_example() {
    let mut cmd = Command::cargo_bin("analyze").unwrap();
    cmd.arg("examples/bad_example.c")
       .arg("--rules")
       .arg("rules.txt");
    
    cmd.assert()
       .success()
       .stdout(predicate::str::contains("gets"))
       .stdout(predicate::str::contains("goto"))
       .stdout(predicate::str::contains("double free"));
}

#[test]
fn test_analyze_good_example() {
    let mut cmd = Command::cargo_bin("analyze").unwrap();
    cmd.arg("examples/good_example.c")
       .arg("--rules")
       .arg("rules.txt");
    
    cmd.assert()
       .success();
    // Good example should have fewer issues
}

#[test]
fn test_analyze_json_output() {
    let mut cmd = Command::cargo_bin("analyze").unwrap();
    cmd.arg("examples/bad_example.c")
       .arg("--format")
       .arg("json")
       .arg("--rules")
       .arg("rules.txt");
    
    cmd.assert()
       .success()
       .stdout(predicate::str::contains("\"file\""))
       .stdout(predicate::str::contains("\"issues\""));
}

#[test]
fn test_analyze_nonexistent_file() {
    let mut cmd = Command::cargo_bin("analyze").unwrap();
    cmd.arg("nonexistent.c");
    
    cmd.assert()
       .failure()
       .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn test_analyze_strict_mode() {
    let mut cmd = Command::cargo_bin("analyze").unwrap();
    cmd.arg("examples/bad_example.c")
       .arg("--strict")
       .arg("--rules")
       .arg("rules.txt");
    
    cmd.assert()
       .failure(); // Should fail because bad_example.c has issues
}

#[test]
fn test_request_simple() {
    let mut cmd = Command::cargo_bin("request").unwrap();
    cmd.arg("Test prompt");
    
    cmd.assert()
       .success()
       .stdout(predicate::str::contains("Mock response"));
}

#[test]
fn test_request_with_file() {
    let mut cmd = Command::cargo_bin("request").unwrap();
    cmd.arg("Analyze this code")
       .arg("--file")
       .arg("examples/bad_example.c");
    
    cmd.assert()
       .success()
       .stdout(predicate::str::contains("Mock analysis response"));
}

#[test]
fn test_request_raw_output() {
    let mut cmd = Command::cargo_bin("request").unwrap();
    cmd.arg("Test prompt")
       .arg("--raw");
    
    cmd.assert()
       .success()
       .stdout(predicate::str::contains("{\"response\""));
}

#[test]
fn test_custom_rules_file() {
    let temp_dir = TempDir::new().unwrap();
    let rules_file = temp_dir.path().join("custom.txt");
    
    fs::write(&rules_file, "test_rule|warning|test|Test rule|Test suggestion\n").unwrap();
    
    let mut cmd = Command::cargo_bin("analyze").unwrap();
    cmd.arg("examples/bad_example.c")
       .arg("--rules")
       .arg(&rules_file);
    
    cmd.assert()
       .success();
}

#[test]
fn test_directory_analysis() {
    let mut cmd = Command::cargo_bin("analyze").unwrap();
    cmd.arg("examples/")
       .arg("--rules")
       .arg("rules.txt");
    
    cmd.assert()
       .success()
       .stdout(predicate::str::contains("bad_example.c"))
       .stdout(predicate::str::contains("good_example.c"));
}