use devit_cli::core::security::workspace::SecureWorkspace;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_cannot_escape_sandbox() {
    let temp = TempDir::new().expect("create sandbox root");
    let root = temp.path();
    fs::create_dir_all(root.join("subdir")).expect("create subdir");
    fs::create_dir_all(root.join("another")).expect("create another");

    let mut workspace = SecureWorkspace::new(root.to_path_buf()).expect("init workspace");

    assert!(workspace
        .change_dir("../../../etc/passwd")
        .expect_err("path traversal should fail")
        .to_string()
        .contains("Security"));
    assert!(workspace
        .resolve_path("/etc/passwd")
        .expect_err("absolute path should fail")
        .to_string()
        .contains("outside sandbox"));
    assert!(workspace
        .change_dir("../..")
        .expect_err("escape via parent should fail")
        .to_string()
        .contains("Security"));

    assert!(workspace.change_dir("subdir").is_ok());
    assert!(workspace.change_dir("../another").is_ok());
    assert!(workspace.change_dir("..").is_ok());
    assert!(workspace.change_dir("./another").is_ok());
}
