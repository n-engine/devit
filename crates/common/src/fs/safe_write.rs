use anyhow::{bail, Context, Result};
use path_clean::PathClean;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub enum WriteMode {
    Overwrite,
    Append,
    CreateNew,
}

pub struct SafeFileWriter {
    allowed_base_dirs: Vec<PathBuf>,
    max_file_size: Option<usize>,
}

impl SafeFileWriter {
    pub fn new() -> Result<Self> {
        let cwd = std::env::current_dir().context("failed to resolve current directory")?;
        Ok(Self {
            allowed_base_dirs: vec![cwd, std::env::temp_dir()],
            max_file_size: Some(10 * 1024 * 1024), // 10MB default max
        })
    }

    pub fn with_allowed_dirs(mut self, dirs: Vec<PathBuf>) -> Self {
        self.allowed_base_dirs = dirs;
        self
    }

    pub fn with_max_size(mut self, max_size: Option<usize>) -> Self {
        self.max_file_size = max_size;
        self
    }

    fn validate_path(&self, path: &Path) -> Result<()> {
        // Clean and normalize the path
        let cleaned_path = path.clean();

        // Reject absolute paths outside of allowed directories
        if cleaned_path.is_absolute() {
            let canonical_path = match cleaned_path.canonicalize() {
                Ok(resolved) => resolved,
                Err(_) => cleaned_path.clone(),
            };

            if !self
                .allowed_base_dirs
                .iter()
                .any(|base| canonical_path.starts_with(base))
            {
                bail!(
                    "absolute path '{}' is outside of allowed directories",
                    path.display()
                );
            }
        } else {
            // For relative paths, check against current directory
            let current_dir =
                std::env::current_dir().context("failed to resolve current directory")?;

            let full_path = current_dir.join(&cleaned_path);
            let canonical_full = full_path.canonicalize().unwrap_or(full_path);

            if !self
                .allowed_base_dirs
                .iter()
                .any(|base| canonical_full.starts_with(base))
            {
                bail!(
                    "relative path '{}' escapes allowed directories",
                    path.display()
                );
            }
        }

        // Check for path traversal attempts
        for component in cleaned_path.components() {
            if let std::path::Component::ParentDir = component {
                bail!(
                    "path '{}' contains parent directory traversal",
                    path.display()
                );
            }
        }

        Ok(())
    }

    fn validate_content(&self, content: &[u8]) -> Result<()> {
        if let Some(max_size) = self.max_file_size {
            if content.len() > max_size {
                bail!(
                    "content size {} exceeds maximum {} bytes",
                    content.len(),
                    max_size
                );
            }
        }
        Ok(())
    }

    pub fn write(&self, path: &Path, content: &[u8], mode: WriteMode) -> Result<()> {
        // Validate path
        self.validate_path(path)?;

        // Validate content
        self.validate_content(content)?;

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create parent directories for '{}'",
                        parent.display()
                    )
                })?;
            }
        }

        // Choose write strategy
        match mode {
            WriteMode::Overwrite => {
                fs::write(path, content)
                    .with_context(|| format!("failed to overwrite file '{}'", path.display()))?;
            }
            WriteMode::Append => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .with_context(|| format!("failed to open '{}' for append", path.display()))?;

                file.write_all(content)
                    .with_context(|| format!("failed to append data into '{}'", path.display()))?;
            }
            WriteMode::CreateNew => {
                // Fail if file exists
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(path)
                    .with_context(|| format!("failed to create new file '{}'", path.display()))?;

                file.write_all(content).with_context(|| {
                    format!(
                        "failed to write data into newly created '{}'",
                        path.display()
                    )
                })?;
            }
        }

        Ok(())
    }

    pub fn write_text(&self, path: &Path, content: &str, mode: WriteMode) -> Result<()> {
        self.write(path, content.as_bytes(), mode)
    }
}
