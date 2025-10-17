//! # Patch fixtures
//!
//! Mock unified diff patches for testing various scenarios

/// Creates a simple function addition patch
pub fn create_simple_function_add_patch() -> String {
    r#"diff --git a/README.md b/README.md
index 0000000..1111111 100644
--- a/README.md
+++ b/README.md
@@ -1,3 +1,4 @@
 # Project

 This is a test project.
+Added a new line for testing.
"#
    .to_string()
}

/// Creates a patch that modifies .env file (protected path)
pub fn create_env_file_patch() -> String {
    r#"diff --git a/.env b/.env
index 1234567..abcdefg 100644
--- a/.env
+++ b/.env
@@ -1,2 +1,3 @@
 API_KEY=secret123
 DEBUG=true
+NEW_SECRET=dangerous
"#
    .to_string()
}

/// Creates a patch adding a small PNG binary file
pub fn create_small_png_patch() -> String {
    // Mock PNG binary content (base64 encoded for example)
    r#"diff --git a/assets/icon.png b/assets/icon.png
new file mode 100644
index 0000000..1234567
Binary files /dev/null and b/assets/icon.png differ
"#
    .to_string()
}

/// Creates a patch modifying .gitmodules (protected infrastructure file)
pub fn create_gitmodules_patch() -> String {
    r#"diff --git a/.gitmodules b/.gitmodules
index 1234567..abcdefg 100644
--- a/.gitmodules
+++ b/.gitmodules
@@ -1,3 +1,3 @@
 [submodule "vendor/lib"]
 	path = vendor/lib
-	url = https://github.com/original/lib.git
+	url = https://github.com/potentially-malicious/lib.git
"#
    .to_string()
}

/// Creates a patch that adds executable permission to a file
pub fn create_exec_bit_patch() -> String {
    r#"diff --git a/script.sh b/script.sh
old mode 100644
new mode 100755
index 1234567..abcdefg 100644
--- a/script.sh
+++ b/script.sh
@@ -1,2 +1,3 @@
 #!/bin/bash
 echo "test"
+echo "new line"
"#
    .to_string()
}

/// Creates a patch that adds a dangerous symlink pointing outside workspace
pub fn create_dangerous_symlink_patch() -> String {
    r#"diff --git a/config/secrets b/config/secrets
new file mode 120000
index 0000000..1234567
--- /dev/null
+++ b/config/secrets
@@ -0,0 +1 @@
+/etc/passwd
\ No newline at end of file
"#
    .to_string()
}

/// Creates a patch with many changes to test line/file limits
pub fn create_large_patch() -> String {
    let mut patch = String::from(
        r#"diff --git a/src/large_change.rs b/src/large_change.rs
index 1234567..abcdefg 100644
--- a/src/large_change.rs
+++ b/src/large_change.rs
@@ -1,5 +1,100 @@
 // Original file content
 use std::collections::HashMap;

"#,
    );

    // Add many lines to exceed moderate policy limits
    for i in 1..=100 {
        patch.push_str(&format!(
            "+// New line {}: fn function_{}() -> i32 {{ {} }}\n",
            i, i, i
        ));
    }

    patch
}

/// Creates a patch affecting multiple files to test file count limits
pub fn create_multi_file_patch() -> String {
    let mut patch = String::new();

    for i in 1..=15 {
        patch.push_str(&format!(
            r#"diff --git a/src/file_{}.rs b/src/file_{}.rs
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/src/file_{}.rs
@@ -0,0 +1,3 @@
+// File {} content
+pub fn function_{}() {{
+}}

"#,
            i, i, i, i, i
        ));
    }

    patch
}

/// Creates a patch with submodule changes that require special handling
pub fn create_submodule_change_patch() -> String {
    r#"diff --git a/vendor/lib b/vendor/lib
index 1234567..abcdefg 160000
--- a/vendor/lib
+++ b/vendor/lib
@@ -1 +1 @@
-Subproject commit 1234567890abcdef1234567890abcdef12345678
+Subproject commit abcdef1234567890abcdef1234567890abcdef12
"#
    .to_string()
}

/// Creates a patch that tries to modify a binary executable
pub fn create_binary_executable_patch() -> String {
    r#"diff --git a/bin/myapp b/bin/myapp
index 1234567..abcdefg 100755
Binary files a/bin/myapp and b/bin/myapp differ
"#
    .to_string()
}
