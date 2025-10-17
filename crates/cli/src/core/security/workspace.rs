use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use path_clean::PathClean;
use tracing::{debug, warn};

/// Secure workspace abstraction enforcing sandbox boundaries and allowlists.
pub struct SecureWorkspace {
    /// Canonical sandbox root directory (jail boundary)
    sandbox_root: PathBuf,
    /// Current working directory relative to the sandbox root
    current_dir: PathBuf,
    /// Optional allowlist of project patterns
    allowed_patterns: Option<GlobSet>,
    /// Raw allowed projects (for diagnostics)
    pub allowed_projects: Vec<String>,
}

impl SecureWorkspace {
    /// Create a new secure workspace rooted at `sandbox_root` (must exist).
    pub fn new(sandbox_root: PathBuf) -> Result<Self> {
        let sandbox_root = sandbox_root.canonicalize().with_context(|| {
            format!(
                "Failed to canonicalize sandbox root: {}",
                sandbox_root.display()
            )
        })?;

        if !sandbox_root.is_dir() {
            bail!(
                "Sandbox root is not a directory: {}",
                sandbox_root.display()
            );
        }

        Ok(Self {
            sandbox_root,
            current_dir: PathBuf::from("."),
            allowed_patterns: None,
            allowed_projects: Vec::new(),
        })
    }

    /// Configure the allowlist of projects (glob syntax, relative to sandbox root).
    pub fn set_allowed_projects(&mut self, projects: &[String]) -> Result<()> {
        self.allowed_projects = projects.to_vec();
        if projects.is_empty() {
            self.allowed_patterns = None;
            return Ok(());
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in projects {
            let normalized = pattern.replace('\\', "/");
            builder.add(Glob::new(&normalized)?);
            if let Some(trimmed) = normalized.strip_suffix("/**") {
                if !trimmed.is_empty() {
                    builder.add(Glob::new(trimmed)?);
                }
            }
        }
        self.allowed_patterns = Some(builder.build()?);
        Ok(())
    }

    /// Change the current working directory (relative to sandbox).
    pub fn change_dir(&mut self, path: &str) -> Result<PathBuf> {
        let cleaned = self.normalize_candidate(path)?;

        if !cleaned.exists() {
            bail!("Directory does not exist: {}", cleaned.display());
        }

        let canonical = cleaned
            .canonicalize()
            .with_context(|| format!("Failed to access directory: {}", cleaned.display()))?;

        self.ensure_within_sandbox(&canonical)?;
        if !canonical.is_dir() {
            bail!("{} is not a directory", canonical.display());
        }

        let relative = self.relative_from_root(&canonical)?;
        self.ensure_allowed(&relative)?;

        debug!(
            "Secure workspace: changed directory to {} (relative: {})",
            canonical.display(),
            relative.display()
        );

        self.current_dir = if relative.components().next().is_none() {
            PathBuf::from(".")
        } else {
            relative
        };

        Ok(canonical)
    }

    /// Resolve a path within the sandbox; non-existent paths are allowed.
    pub fn resolve_path(&self, path: &str) -> Result<PathBuf> {
        let cleaned = self.normalize_candidate(path)?;

        if cleaned.exists() {
            let canonical = cleaned
                .canonicalize()
                .with_context(|| format!("Failed to canonicalize path: {}", cleaned.display()))?;
            self.ensure_within_sandbox(&canonical)?;
            return Ok(canonical);
        }

        self.ensure_normalized(&cleaned)?;
        Ok(cleaned)
    }

    /// Current working directory (absolute path).
    pub fn current_dir(&self) -> PathBuf {
        self.sandbox_root.join(&self.current_dir)
    }

    /// Current working directory relative to sandbox root.
    pub fn current_relative(&self) -> PathBuf {
        self.current_dir.clone()
    }

    /// Resolve a path relative to the sandbox root without changing state.
    pub fn resolve_relative_from_root(&self, path: &str) -> Result<PathBuf> {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            bail!("Absolute paths are not allowed in workspace operations");
        }

        let absolute = self.sandbox_root.join(candidate).clean();
        self.ensure_normalized(&absolute)?;
        let relative = self.relative_from_root(&absolute)?;
        self.ensure_allowed(&relative)?;
        Ok(relative)
    }

    fn normalize_candidate(&self, path: &str) -> Result<PathBuf> {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            bail!("Absolute paths are not allowed in workspace operations");
        }

        Ok(self
            .sandbox_root
            .join(&self.current_dir)
            .join(candidate)
            .clean())
    }

    fn ensure_within_sandbox(&self, path: &Path) -> Result<()> {
        if !path.starts_with(&self.sandbox_root) {
            warn!(
                "Secure workspace violation: {} outside sandbox {}",
                path.display(),
                self.sandbox_root.display()
            );
            bail!("Security violation: attempted to escape sandbox");
        }
        Ok(())
    }

    fn ensure_normalized(&self, path: &Path) -> Result<()> {
        if !path.starts_with(&self.sandbox_root) {
            bail!("Security violation: path outside sandbox");
        }

        if let Ok(relative) = path.strip_prefix(&self.sandbox_root) {
            let mut depth = 0i32;
            for component in relative.components() {
                match component {
                    Component::Prefix(_) => bail!("Unsupported path prefix"),
                    Component::RootDir | Component::CurDir => {}
                    Component::ParentDir => {
                        depth -= 1;
                        if depth < 0 {
                            bail!("Security violation: path escapes sandbox");
                        }
                    }
                    Component::Normal(_) => {
                        depth += 1;
                    }
                }
            }
        }

        Ok(())
    }

    fn relative_from_root(&self, path: &Path) -> Result<PathBuf> {
        let relative = path.strip_prefix(&self.sandbox_root).with_context(|| {
            format!(
                "Path {} not within sandbox {}",
                path.display(),
                self.sandbox_root.display()
            )
        })?;
        Ok(relative.to_path_buf())
    }

    fn ensure_allowed(&self, relative: &Path) -> Result<()> {
        let Some(patterns) = &self.allowed_patterns else {
            return Ok(());
        };

        if relative.components().next().is_none() {
            // Root always allowed
            return Ok(());
        }

        let rel_str = relative.to_string_lossy().replace('\\', "/");
        if patterns.is_match(rel_str.as_str()) {
            return Ok(());
        }

        bail!(
            "Working directory '{}' is not in the allowed project list",
            rel_str
        );
    }
}

#[cfg(test)]
mod tests {
    use super::SecureWorkspace;
    use tempfile::TempDir;

    #[test]
    fn change_dir_stays_in_sandbox() {
        let temp = TempDir::new().unwrap();
        let sandbox = temp.path().to_path_buf();
        std::fs::create_dir_all(sandbox.join("project-a/sub")).unwrap();

        let mut ws = SecureWorkspace::new(sandbox.clone()).unwrap();
        ws.set_allowed_projects(&["project-a/**".to_string(), "project-b".to_string()])
            .unwrap();

        let new_dir = ws.change_dir("project-a").unwrap();
        assert_eq!(new_dir, sandbox.join("project-a"));

        let nested = ws.change_dir("sub").unwrap();
        assert_eq!(nested, sandbox.join("project-a/sub"));

        assert!(ws.change_dir("../../etc").is_err());
        assert!(ws.change_dir("../project-b").is_err());
    }

    #[test]
    fn resolve_path_prevents_escape() {
        let temp = TempDir::new().unwrap();
        let sandbox = temp.path().to_path_buf();
        let ws = SecureWorkspace::new(sandbox.clone()).unwrap();

        assert!(ws.resolve_path("../../etc/passwd").is_err());
        let file = ws.resolve_path("project/file.txt").unwrap();
        assert!(file.starts_with(&sandbox));
    }

    #[test]
    fn resolve_relative_respects_allowlist() {
        let temp = TempDir::new().unwrap();
        let sandbox = temp.path().to_path_buf();
        std::fs::create_dir_all(sandbox.join("project-b")).unwrap();

        let mut ws = SecureWorkspace::new(sandbox).unwrap();
        ws.set_allowed_projects(&["project-a/**".to_string()])
            .unwrap();

        assert!(ws.resolve_relative_from_root("project-a").is_ok());
        assert!(ws.resolve_relative_from_root("project-b").is_err());
    }
}
