use crate::core::{DevItError, DevItResult};
use path_clean::PathClean;
use std::fs;
use std::path::{Path, PathBuf};

/// Security module for path validation and symlink protection
///
/// This module implements C4 security requirements:
/// - Strict path canonicalization without following symlinks outside repo
/// - Symlink validation to prevent escaping repository boundaries
/// - Path traversal attack prevention

#[derive(Debug, Clone)]
pub struct PathSecurityContext {
    /// The repository root path (canonicalized)
    repo_root: PathBuf,
    /// Whether to allow relative symlinks within the repository
    allow_internal_symlinks: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathSecurityViolation {
    /// Path escapes repository boundaries via '..' or absolute path
    RepositoryEscape {
        path: PathBuf,
        canonical_path: PathBuf,
    },
    /// Symlink points to absolute path
    SymlinkAbsoluteTarget { symlink: PathBuf, target: PathBuf },
    /// Symlink escapes repository boundaries
    SymlinkEscapesRepo { symlink: PathBuf, target: PathBuf },
    /// Path traversal attempt detected
    PathTraversal { path: PathBuf },
    /// Invalid or non-existent path
    InvalidPath { path: PathBuf, error: String },
}

impl PathSecurityContext {
    /// Create a new path security context for the given repository root
    pub fn new<P: AsRef<Path>>(repo_root: P, allow_internal_symlinks: bool) -> DevItResult<Self> {
        let repo_root = fs::canonicalize(repo_root.as_ref()).map_err(|e| {
            DevItError::io(
                Some(repo_root.as_ref().to_path_buf()),
                "canonicalize repository root",
                e,
            )
        })?;

        Ok(Self {
            repo_root,
            allow_internal_symlinks,
        })
    }

    /// Validate a file path from a patch, ensuring it's safe to modify
    pub fn validate_patch_path<P: AsRef<Path>>(&self, path: P) -> DevItResult<PathBuf> {
        let path = path.as_ref();

        // Special case: /dev/null is used in patches for file creation/deletion
        if path == Path::new("/dev/null") {
            return Ok(PathBuf::from("/dev/null"));
        }

        // First, perform basic path validation
        self.validate_basic_path(path)?;

        // Convert to absolute path relative to repo root
        let full_path = if path.is_absolute() {
            // Absolute paths are generally suspicious in patches
            return Err(DevItError::PolicyBlock {
                rule: "path_security_no_absolute".to_string(),
                required_level: "any".to_string(),
                current_level: "patch".to_string(),
                context: format!("Absolute paths not allowed in patches: {}", path.display()),
            });
        } else {
            self.repo_root.join(path)
        };

        // Canonicalize the path to resolve any '..' or '.' components
        let canonical_path = self.safe_canonicalize(&full_path)?;

        // Ensure the canonical path is within repository boundaries
        if !canonical_path.starts_with(&self.repo_root) {
            return Err(DevItError::PolicyBlock {
                rule: "path_security_repo_boundary".to_string(),
                required_level: "any".to_string(),
                current_level: "patch".to_string(),
                context: format!(
                    "Path escapes repository: {} -> {}",
                    path.display(),
                    canonical_path.display()
                ),
            });
        }

        Ok(canonical_path)
    }

    /// Validate a symlink, checking both the symlink path and its target
    pub fn validate_symlink<P: AsRef<Path>, T: AsRef<Path>>(
        &self,
        symlink_path: P,
        target: T,
    ) -> DevItResult<()> {
        let symlink_path = symlink_path.as_ref();
        let target = target.as_ref();

        // Validate the symlink path itself
        let canonical_symlink = self.validate_patch_path(symlink_path)?;

        // Check the target
        if target.is_absolute() {
            return Err(DevItError::PolicyBlock {
                rule: "symlink_security_no_absolute".to_string(),
                required_level: "any".to_string(),
                current_level: "patch".to_string(),
                context: format!(
                    "Symlink {} points to absolute path: {}",
                    symlink_path.display(),
                    target.display()
                ),
            });
        }

        // Resolve target relative to symlink's directory
        let symlink_dir = canonical_symlink.parent().unwrap_or(&self.repo_root);
        let target_path = symlink_dir.join(target);
        let canonical_target = self.safe_canonicalize(&target_path)?;

        // Ensure target is within repository boundaries
        if !canonical_target.starts_with(&self.repo_root) {
            return Err(DevItError::PolicyBlock {
                rule: "symlink_security_repo_boundary".to_string(),
                required_level: "any".to_string(),
                current_level: "patch".to_string(),
                context: format!(
                    "Symlink {} target escapes repository: {} -> {}",
                    symlink_path.display(),
                    target.display(),
                    canonical_target.display()
                ),
            });
        }

        // Check policy for internal symlinks
        if !self.allow_internal_symlinks {
            return Err(DevItError::PolicyBlock {
                rule: "symlink_policy_disabled".to_string(),
                required_level: "trusted".to_string(),
                current_level: "patch".to_string(),
                context: "Symlinks not allowed by current policy".to_string(),
            });
        }

        Ok(())
    }

    /// Perform pre-commit validation of all paths to prevent TOCTOU attacks
    pub fn pre_commit_validation<P: AsRef<Path>>(&self, paths: &[P]) -> DevItResult<()> {
        for path in paths {
            let path = path.as_ref();

            // Re-validate each path
            let canonical_path = self.validate_patch_path(path)?;

            // Check if path exists and is still safe
            if canonical_path.exists() {
                // Check if it became a symlink after initial validation
                if canonical_path.is_symlink() {
                    let target = fs::read_link(&canonical_path).map_err(|e| {
                        DevItError::io(Some(canonical_path.clone()), "read symlink", e)
                    })?;

                    self.validate_symlink(&canonical_path, &target)?;
                }
            }
        }

        Ok(())
    }

    /// Validate basic path properties (no path traversal, valid characters)
    fn validate_basic_path<P: AsRef<Path>>(&self, path: P) -> DevItResult<()> {
        let path = path.as_ref();
        let path_str = path.to_string_lossy();

        // Check for obvious path traversal attempts
        if path_str.contains("../") || path_str.contains("..\\") {
            return Err(DevItError::PolicyBlock {
                rule: "path_traversal_protection".to_string(),
                required_level: "any".to_string(),
                current_level: "patch".to_string(),
                context: "Path traversal attempt detected".to_string(),
            });
        }

        // Check for null bytes or other suspicious characters
        if path_str.contains('\0') {
            return Err(DevItError::PolicyBlock {
                rule: "path_security_null_byte".to_string(),
                required_level: "any".to_string(),
                current_level: "patch".to_string(),
                context: "Null byte in path".to_string(),
            });
        }

        // Check for control characters (except tab and newline)
        if path_str
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n')
        {
            return Err(DevItError::PolicyBlock {
                rule: "path_security_control_chars".to_string(),
                required_level: "any".to_string(),
                current_level: "patch".to_string(),
                context: "Control character in path".to_string(),
            });
        }

        // Check for extremely long paths
        if path_str.len() > 4096 {
            return Err(DevItError::PolicyBlock {
                rule: "path_security_length_limit".to_string(),
                required_level: "any".to_string(),
                current_level: "patch".to_string(),
                context: "Path too long".to_string(),
            });
        }

        Ok(())
    }

    /// Safe canonicalization that doesn't follow symlinks outside repo
    fn safe_canonicalize<P: AsRef<Path>>(&self, path: P) -> DevItResult<PathBuf> {
        let path = path.as_ref();

        // If path doesn't exist, we can't canonicalize it fully
        // Instead, we'll canonicalize the existing parent and append the filename
        if !path.exists() {
            if let Some(parent) = path.parent() {
                if parent.exists() {
                    let canonical_parent = fs::canonicalize(parent).map_err(|e| {
                        DevItError::io(Some(parent.to_path_buf()), "canonicalize parent path", e)
                    })?;

                    if let Some(filename) = path.file_name() {
                        return Ok(canonical_parent.join(filename));
                    }
                }
            }
            // If no parent exists, resolve manually
            return self.manual_path_resolution(path);
        }

        // Path exists, but we need to be careful about symlinks
        let canonical = fs::canonicalize(path)
            .map_err(|e| DevItError::io(Some(path.to_path_buf()), "canonicalize path", e))?;

        Ok(canonical)
    }

    /// Manual path resolution for non-existent paths
    fn manual_path_resolution<P: AsRef<Path>>(&self, path: P) -> DevItResult<PathBuf> {
        let path = path.as_ref();

        if path.is_absolute() {
            return Ok(path.clean());
        }

        let mut resolved = self.repo_root.clone();

        for component in path.components() {
            match component {
                std::path::Component::Normal(name) => {
                    resolved.push(name);
                }
                std::path::Component::ParentDir => {
                    if !resolved.pop() || !resolved.starts_with(&self.repo_root) {
                        return Err(DevItError::PolicyBlock {
                            rule: "path_resolution_escape".to_string(),
                            required_level: "any".to_string(),
                            current_level: "patch".to_string(),
                            context: "Path resolution would escape repository".to_string(),
                        });
                    }
                }
                std::path::Component::CurDir => {
                    // Skip current directory references
                }
                std::path::Component::RootDir => {
                    // Skip root directory component (absolute paths start with this)
                    // This happens when resolving absolute paths manually
                }
                std::path::Component::Prefix(_) => {
                    // Skip Windows drive prefixes (C:, D:, etc.)
                    // This should not occur on Unix systems but handle it gracefully
                }
            }
        }

        Ok(resolved)
    }

    /// Get the repository root
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_repo() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path().to_path_buf();

        // Create some test directories
        fs::create_dir_all(repo_root.join("src")).unwrap();
        fs::create_dir_all(repo_root.join("tests")).unwrap();

        (temp_dir, repo_root)
    }

    #[test]
    fn test_valid_paths() {
        let (_temp, repo_root) = create_test_repo();
        let security = PathSecurityContext::new(&repo_root, true).unwrap();

        // Valid relative paths
        assert!(security.validate_patch_path("src/main.rs").is_ok());
        assert!(security.validate_patch_path("tests/test.rs").is_ok());
        assert!(security.validate_patch_path("README.md").is_ok());
    }

    #[test]
    fn test_path_traversal_blocked() {
        let (_temp, repo_root) = create_test_repo();
        let security = PathSecurityContext::new(&repo_root, true).unwrap();

        // Path traversal attempts should be blocked
        assert!(security.validate_patch_path("../etc/passwd").is_err());
        assert!(security
            .validate_patch_path("src/../../etc/passwd")
            .is_err());
        assert!(security.validate_patch_path("./../../etc/passwd").is_err());
    }

    #[test]
    fn test_absolute_paths_blocked() {
        let (_temp, repo_root) = create_test_repo();
        let security = PathSecurityContext::new(&repo_root, true).unwrap();

        // Absolute paths should be blocked
        assert!(security.validate_patch_path("/etc/passwd").is_err());
        assert!(security.validate_patch_path("/tmp/test").is_err());
    }

    #[test]
    fn test_symlink_validation() {
        let (_temp, repo_root) = create_test_repo();
        let security = PathSecurityContext::new(&repo_root, true).unwrap();

        // Valid internal symlink
        assert!(security
            .validate_symlink("src/link.rs", "../tests/test.rs")
            .is_ok());

        // Invalid absolute symlink
        assert!(security
            .validate_symlink("src/link.rs", "/etc/passwd")
            .is_err());

        // Invalid escaping symlink
        assert!(security
            .validate_symlink("src/link.rs", "../../etc/passwd")
            .is_err());
    }

    #[test]
    fn test_symlink_policy_disabled() {
        let (_temp, repo_root) = create_test_repo();
        let security = PathSecurityContext::new(&repo_root, false).unwrap();

        // All symlinks should be blocked when policy disabled
        assert!(security
            .validate_symlink("src/link.rs", "../tests/test.rs")
            .is_err());
    }

    #[test]
    fn test_malicious_characters() {
        let (_temp, repo_root) = create_test_repo();
        let security = PathSecurityContext::new(&repo_root, true).unwrap();

        // Null bytes should be blocked
        assert!(security.validate_patch_path("src/test\0.rs").is_err());

        // Extremely long paths should be blocked
        let long_path = "a".repeat(5000);
        assert!(security.validate_patch_path(&long_path).is_err());
    }
}
