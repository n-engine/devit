//! # DevIt File Operations Module
//!
//! This module provides secure file exploration capabilities for MCP integration.
//! All operations respect path security, .gitignore rules, and size limits.

use crate::core::formats::{Compressible, FieldMappings, FormatUtils, OutputFormat};
use crate::core::{DevItError, DevItResult};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::info;
use uuid;

/// Maximum file size that can be read (1MB)
const MAX_FILE_SIZE: u64 = 1024 * 1024;

/// Maximum number of search results
const MAX_SEARCH_RESULTS: usize = 100;

/// Maximum tree depth for project structure
const MAX_TREE_DEPTH: u8 = 10;

/// File entry with metadata for listing operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// File or directory name
    pub name: String,
    /// Full path relative to project root
    pub path: PathBuf,
    /// Entry type (file, directory, symlink)
    pub entry_type: FileType,
    /// File size in bytes (None for directories)
    pub size: Option<u64>,
    /// Last modified timestamp
    pub modified: Option<SystemTime>,
    /// File permissions (readable, writable, executable)
    pub permissions: FilePermissions,
}

/// File type enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    File,
    Directory,
    Symlink,
}

/// File permissions information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePermissions {
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}

/// File content with optional line numbers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    /// File path
    pub path: PathBuf,
    /// File content as string
    pub content: String,
    /// File size in bytes
    pub size: u64,
    /// Content with line numbers if requested
    pub lines: Option<Vec<String>>,
    /// File encoding detected
    pub encoding: String,
}

/// Search result entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    /// File path where match was found
    pub file: PathBuf,
    /// Line number (1-indexed)
    pub line_number: usize,
    /// Matching line content
    pub line: String,
    /// Context lines before match
    pub context_before: Vec<String>,
    /// Context lines after match
    pub context_after: Vec<String>,
}

/// Search results collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    /// Search pattern used
    pub pattern: String,
    /// Search path
    pub path: PathBuf,
    /// Number of files searched
    pub files_searched: usize,
    /// Total matches found
    pub total_matches: usize,
    /// Match results (limited to MAX_SEARCH_RESULTS)
    pub matches: Vec<SearchMatch>,
    /// Whether results were truncated
    pub truncated: bool,
}

/// Project structure tree node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    /// Node name
    pub name: String,
    /// Full path
    pub path: PathBuf,
    /// Node type
    pub node_type: FileType,
    /// Child nodes (None for files)
    pub children: Option<Vec<TreeNode>>,
    /// File size for files
    pub size: Option<u64>,
}

/// Project structure overview
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStructure {
    /// Root path
    pub root: PathBuf,
    /// Detected project type
    pub project_type: Option<String>,
    /// Tree structure
    pub tree: TreeNode,
    /// Total files count
    pub total_files: usize,
    /// Total directories count
    pub total_dirs: usize,
}

/// File operations manager with security validation
pub struct FileOpsManager {
    /// Project root path
    root_path: PathBuf,
    /// Path security context for validation
    path_security: crate::core::path_security::PathSecurityContext,
    /// Whether internal symlinks are allowed for this manager
    allow_internal_symlinks: bool,
}

impl FileOpsManager {
    /// Create new file operations manager with automatic project root detection
    pub fn new<P: AsRef<Path>>(root_path: P) -> DevItResult<Self> {
        let allow_internal_symlinks = false;
        let (final_root, path_security) = Self::prepare_root(root_path, allow_internal_symlinks)?;

        Ok(Self {
            root_path: final_root,
            path_security,
            allow_internal_symlinks,
        })
    }

    /// Update the root path while preserving allowlist configuration.
    pub fn set_root_path<P: AsRef<Path>>(&mut self, root_path: P) -> DevItResult<()> {
        let (final_root, path_security) =
            Self::prepare_root(root_path, self.allow_internal_symlinks)?;
        self.root_path = final_root;
        self.path_security = path_security;
        Ok(())
    }

    fn prepare_root<P: AsRef<Path>>(
        root_path: P,
        allow_internal_symlinks: bool,
    ) -> DevItResult<(PathBuf, crate::core::path_security::PathSecurityContext)> {
        let provided_path = root_path.as_ref().to_path_buf();
        let forced_root = env::var("DEVIT_FORCE_ROOT").ok().map(PathBuf::from);

        let final_root = if let Some(force) = forced_root {
            let canonical = force
                .canonicalize()
                .map_err(|err| DevItError::io(Some(force.clone()), "prepare_root", err))?;
            info!("DEVIT_FORCE_ROOT applied: {}", canonical.display());
            canonical
        } else {
            let detected_root = Self::auto_detect_project_root(&provided_path);
            detected_root.unwrap_or(provided_path)
        };
        let path_security = crate::core::path_security::PathSecurityContext::new(
            &final_root,
            allow_internal_symlinks,
        )?;
        Ok((final_root, path_security))
    }

    fn adjust_user_path<'a>(&self, path: &'a Path) -> Cow<'a, Path> {
        if path.is_absolute() {
            if let Ok(stripped) = path.strip_prefix(&self.root_path) {
                if stripped.as_os_str().is_empty() {
                    return Cow::Owned(PathBuf::from("."));
                }
                return Cow::Owned(stripped.to_path_buf());
            }
        }

        Cow::Borrowed(path)
    }

    /// Auto-detect project root by looking for common project indicators
    fn auto_detect_project_root(start_path: &Path) -> Option<PathBuf> {
        // Project indicators in order of priority
        let project_indicators = [
            // Version control (highest priority)
            ".git",
            // Rust
            "Cargo.toml",
            // Node.js/JavaScript
            "package.json",
            // Python
            "pyproject.toml",
            "setup.py",
            "requirements.txt",
            "poetry.lock",
            "Pipfile",
            // Java
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            // Go
            "go.mod",
            "go.sum",
            // C/C++
            "CMakeLists.txt",
            "Makefile",
            "configure.ac",
            // .NET
            "*.sln",
            "*.csproj",
            "*.fsproj",
            "*.vbproj",
            // Ruby
            "Gemfile",
            "Rakefile",
            // PHP
            "composer.json",
            // Dart/Flutter
            "pubspec.yaml",
            // Swift
            "Package.swift",
            // Kotlin
            "build.gradle.kts",
            // Scala
            "build.sbt",
            // Elixir
            "mix.exs",
            // Containerization and DevOps
            ".devcontainer",
            "docker-compose.yml",
            "docker-compose.yaml",
            "Dockerfile",
            // CI/CD
            ".github",
            ".gitlab-ci.yml",
            ".travis.yml",
            "Jenkinsfile",
            // General project files
            "README.md",
            "README.rst",
            "README.txt",
        ];

        // Start from the provided path and walk up the directory tree
        let search_start = if start_path.is_file() {
            start_path.parent().unwrap_or(start_path)
        } else {
            start_path
        };

        for ancestor in search_start.ancestors() {
            for indicator in &project_indicators {
                let indicator_path = ancestor.join(indicator);

                // Handle glob patterns like *.sln
                if indicator.contains('*') {
                    if let Ok(entries) = fs::read_dir(ancestor) {
                        for entry in entries.flatten() {
                            let file_name = entry.file_name();
                            let file_name_str = file_name.to_string_lossy();

                            // Simple glob matching for *.extension
                            if indicator.starts_with('*') {
                                let extension = &indicator[1..];
                                if file_name_str.ends_with(extension) {
                                    return Some(ancestor.to_path_buf());
                                }
                            }
                        }
                    }
                } else if indicator_path.exists() {
                    return Some(ancestor.to_path_buf());
                }
            }

            // Stop at filesystem root
            if ancestor.parent().is_none() {
                break;
            }
        }

        None
    }

    /// Get the current working root directory
    pub fn get_root_path(&self) -> &Path {
        &self.root_path
    }

    /// Read file content with security validation
    pub async fn file_read<P: AsRef<Path>>(
        &self,
        path: P,
        line_numbers: bool,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> DevItResult<FileContent> {
        let path = path.as_ref();
        let user_path = self.adjust_user_path(path);

        // Validate path security
        let validated_path = self.path_security.validate_patch_path(user_path.as_ref())?;

        // Check if file exists and is readable
        if !validated_path.exists() {
            return Err(DevItError::io(
                Some(validated_path.clone()),
                "read file",
                std::io::Error::new(std::io::ErrorKind::NotFound, "File not found"),
            ));
        }

        if !validated_path.is_file() {
            return Err(DevItError::InvalidDiff {
                reason: "Path is not a file".to_string(),
                line_number: None,
            });
        }

        // Check file size
        let metadata = fs::metadata(&validated_path)
            .map_err(|e| DevItError::io(Some(validated_path.clone()), "read file metadata", e))?;

        let file_size = metadata.len();
        if file_size > MAX_FILE_SIZE {
            return Err(DevItError::InvalidDiff {
                reason: format!(
                    "File too large: {} bytes (max: {} bytes)",
                    file_size, MAX_FILE_SIZE
                ),
                line_number: None,
            });
        }

        // Read file content
        let content = fs::read_to_string(&validated_path)
            .map_err(|e| DevItError::io(Some(validated_path.clone()), "read file content", e))?;

        // Apply offset/limit if specified
        let final_content = if let (Some(offset), Some(limit)) = (offset, limit) {
            let lines: Vec<&str> = content.lines().collect();
            let start = offset.min(lines.len());
            let end = (offset + limit).min(lines.len());
            lines[start..end].join("\n")
        } else {
            content
        };

        // Generate line numbers if requested
        let lines = if line_numbers {
            Some(
                final_content
                    .lines()
                    .enumerate()
                    .map(|(i, line)| format!("{:4}: {}", i + 1, line))
                    .collect(),
            )
        } else {
            None
        };

        // Simple encoding detection based on content
        let encoding = if final_content.bytes().take(1000).any(|b| b > 127) {
            if final_content.starts_with("\u{FEFF}") {
                "utf-8-bom"
            } else {
                "utf-8" // Assume UTF-8 for other non-ASCII
            }
        } else {
            "utf-8" // ASCII-compatible, assume UTF-8
        }
        .to_string();

        Ok(FileContent {
            path: user_path.as_ref().to_path_buf(),
            content: final_content,
            size: file_size,
            lines,
            encoding,
        })
    }

    /// List files and directories with metadata
    pub async fn file_list<P: AsRef<Path>>(
        &self,
        path: P,
        recursive: bool,
    ) -> DevItResult<Vec<FileEntry>> {
        let path = path.as_ref();
        let user_path = self.adjust_user_path(path);

        // Validate path security
        let validated_path = self.path_security.validate_patch_path(user_path.as_ref())?;

        if !validated_path.exists() {
            return Err(DevItError::io(
                Some(validated_path.clone()),
                "list directory",
                std::io::Error::new(std::io::ErrorKind::NotFound, "Path not found"),
            ));
        }

        let mut entries = Vec::new();

        if validated_path.is_file() {
            // If it's a single file, return just that file
            if let Ok(entry) = self.create_file_entry(&validated_path) {
                entries.push(entry);
            }
        } else if validated_path.is_dir() {
            // List directory contents
            if recursive {
                self.collect_entries_recursive(&validated_path, &mut entries)?;
            } else {
                self.collect_entries_single(&validated_path, &mut entries)?;
            }
        }

        // Sort entries by name for consistent output
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(entries)
    }

    /// Create a FileEntry from a path
    fn create_file_entry(&self, path: &Path) -> DevItResult<FileEntry> {
        let metadata = fs::metadata(path)
            .map_err(|e| DevItError::io(Some(path.to_path_buf()), "read file metadata", e))?;

        let name = path
            .file_name()
            .unwrap_or_else(|| path.as_os_str())
            .to_string_lossy()
            .to_string();

        let entry_type = if metadata.is_dir() {
            FileType::Directory
        } else if metadata.is_symlink() {
            FileType::Symlink
        } else {
            FileType::File
        };

        let size = if metadata.is_file() {
            Some(metadata.len())
        } else {
            None
        };

        let modified = metadata.modified().ok();

        // Get permissions (cross-platform)
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            let mode = metadata.permissions().mode();
            FilePermissions {
                readable: mode & 0o444 != 0,
                writable: mode & 0o222 != 0,
                executable: mode & 0o111 != 0,
            }
        };

        #[cfg(not(unix))]
        let permissions = FilePermissions {
            readable: true, // Windows: assume readable if we can access it
            writable: !metadata.permissions().readonly(),
            executable: path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| matches!(ext.to_lowercase().as_str(), "exe" | "bat" | "cmd" | "com"))
                .unwrap_or(false),
        };

        Ok(FileEntry {
            name,
            path: path.to_path_buf(),
            entry_type,
            size,
            modified,
            permissions,
        })
    }

    /// Collect entries from a single directory (non-recursive)
    fn collect_entries_single(
        &self,
        dir_path: &Path,
        entries: &mut Vec<FileEntry>,
    ) -> DevItResult<()> {
        let read_dir = fs::read_dir(dir_path)
            .map_err(|e| DevItError::io(Some(dir_path.to_path_buf()), "read directory", e))?;

        for entry in read_dir {
            let entry = entry.map_err(|e| {
                DevItError::io(Some(dir_path.to_path_buf()), "read directory entry", e)
            })?;

            let entry_path = entry.path();

            // Apply filtering rules
            if !self.should_include_path(&entry_path) {
                continue;
            }

            if let Ok(file_entry) = self.create_file_entry(&entry_path) {
                entries.push(file_entry);
            }
        }

        Ok(())
    }

    /// Collect entries recursively from a directory
    fn collect_entries_recursive(
        &self,
        dir_path: &Path,
        entries: &mut Vec<FileEntry>,
    ) -> DevItResult<()> {
        self.collect_entries_single(dir_path, entries)?;

        // Recursively process subdirectories
        let read_dir = fs::read_dir(dir_path)
            .map_err(|e| DevItError::io(Some(dir_path.to_path_buf()), "read directory", e))?;

        for entry in read_dir {
            let entry = entry.map_err(|e| {
                DevItError::io(Some(dir_path.to_path_buf()), "read directory entry", e)
            })?;

            let entry_path = entry.path();

            if entry_path.is_dir() && self.should_include_path(&entry_path) {
                self.collect_entries_recursive(&entry_path, entries)?;
            }
        }

        Ok(())
    }

    /// Search for pattern in files with context lines
    pub async fn file_search<P: AsRef<Path>>(
        &self,
        pattern: &str,
        path: P,
        context_lines: Option<usize>,
    ) -> DevItResult<SearchResults> {
        let path = path.as_ref();
        let user_path = self.adjust_user_path(path);
        let context_lines = context_lines.unwrap_or(2);

        // Validate path security
        let validated_path = self.path_security.validate_patch_path(user_path.as_ref())?;

        if !validated_path.exists() {
            return Err(DevItError::io(
                Some(validated_path.clone()),
                "search files",
                std::io::Error::new(std::io::ErrorKind::NotFound, "Path not found"),
            ));
        }

        // Compile regex pattern
        let regex = regex::Regex::new(pattern).map_err(|e| DevItError::InvalidDiff {
            reason: format!("Invalid regex pattern: {}", e),
            line_number: None,
        })?;

        let mut results = SearchResults {
            pattern: pattern.to_string(),
            path: user_path.as_ref().to_path_buf(),
            files_searched: 0,
            total_matches: 0,
            matches: Vec::new(),
            truncated: false,
        };

        if validated_path.is_file() {
            // Search single file
            self.search_file(&validated_path, &regex, context_lines, &mut results)?;
        } else if validated_path.is_dir() {
            // Search directory recursively
            self.search_directory_recursive(&validated_path, &regex, context_lines, &mut results)?;
        }

        // Check if results were truncated
        if results.matches.len() >= MAX_SEARCH_RESULTS {
            results.matches.truncate(MAX_SEARCH_RESULTS);
            results.truncated = true;
        }

        Ok(results)
    }

    /// Search a single file for the pattern
    fn search_file(
        &self,
        file_path: &Path,
        regex: &regex::Regex,
        context_lines: usize,
        results: &mut SearchResults,
    ) -> DevItResult<()> {
        // Skip files that shouldn't be searched
        if !self.should_search_file(file_path) {
            return Ok(());
        }

        results.files_searched += 1;

        // Read file content
        let content = match fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(_) => return Ok(()), // Skip files that can't be read as text
        };

        let lines: Vec<&str> = content.lines().collect();

        // Search for matches
        for (line_idx, line) in lines.iter().enumerate() {
            if regex.is_match(line) {
                results.total_matches += 1;

                // Collect context lines
                let start_context = if line_idx >= context_lines {
                    line_idx - context_lines
                } else {
                    0
                };
                let end_context = (line_idx + context_lines + 1).min(lines.len());

                let context_before = lines[start_context..line_idx]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();

                let context_after = lines[line_idx + 1..end_context]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();

                results.matches.push(SearchMatch {
                    file: file_path.to_path_buf(),
                    line_number: line_idx + 1, // 1-indexed
                    line: line.to_string(),
                    context_before,
                    context_after,
                });

                // Stop if we've reached the maximum number of results
                if results.matches.len() >= MAX_SEARCH_RESULTS {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Search directory recursively
    fn search_directory_recursive(
        &self,
        dir_path: &Path,
        regex: &regex::Regex,
        context_lines: usize,
        results: &mut SearchResults,
    ) -> DevItResult<()> {
        let read_dir = fs::read_dir(dir_path).map_err(|e| {
            DevItError::io(Some(dir_path.to_path_buf()), "read directory for search", e)
        })?;

        for entry in read_dir {
            let entry = entry.map_err(|e| {
                DevItError::io(
                    Some(dir_path.to_path_buf()),
                    "read directory entry for search",
                    e,
                )
            })?;

            let entry_path = entry.path();

            // Skip paths that shouldn't be included
            if !self.should_include_path(&entry_path) {
                continue;
            }

            if entry_path.is_file() {
                self.search_file(&entry_path, regex, context_lines, results)?;
            } else if entry_path.is_dir() {
                self.search_directory_recursive(&entry_path, regex, context_lines, results)?;
            }

            // Stop early if we've reached maximum results
            if results.matches.len() >= MAX_SEARCH_RESULTS {
                break;
            }
        }

        Ok(())
    }

    /// Check if a file should be searched (filter by extension and type)
    fn should_search_file(&self, path: &Path) -> bool {
        // Get file extension
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

        // Only search text-based files
        match extension.to_lowercase().as_str() {
            // Programming languages
            "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "java" | "c" | "cpp" | "cc" | "cxx"
            | "h" | "hpp" | "cs" | "go" | "php" | "rb" | "swift" | "kt" | "scala" | "clj"
            | "hs" | "ml" | "fs" | "elm" | "dart" | "lua" | "perl" | "r" | "jl" | "nim" | "cr"
            | "zig" | "odin" | "v" | "mod" | "vb" | "pas" | "ada" | "f90" | "f95" | "for"
            | "cob" | "asm" | "s" => true,

            // Markup and config
            "html" | "htm" | "xml" | "xhtml" | "svg" | "css" | "scss" | "sass" | "less"
            | "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "config"
            | "properties" | "env" => true,

            // Documentation
            "md" | "markdown" | "rst" | "txt" | "text" | "doc" | "rtf" | "tex" | "latex" => true,

            // Shell and scripts
            "sh" | "bash" | "zsh" | "fish" | "csh" | "tcsh" | "ksh" | "ps1" | "bat" | "cmd" => true,

            // Other text files
            "log" | "csv" | "tsv" | "sql" | "dockerfile" | "makefile" | "cmake" | "bazel"
            | "gradle" | "sbt" | "pom" | "gemfile" | "podfile" | "rakefile" | "vagrantfile" => true,

            // Files without extension (often config or scripts)
            "" => {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");

                match filename.to_lowercase().as_str() {
                    "dockerfile" | "makefile" | "rakefile" | "gemfile" | "podfile"
                    | "vagrantfile" | "justfile" | "cargo.lock" | "readme" | "license"
                    | "changelog" | "contributing" | "authors" | "credits" | "copying"
                    | "install" | "news" | "thanks" | "todo" | "bugs" | "history" => true,
                    _ => false,
                }
            }

            _ => false,
        }
    }

    /// Generate project structure tree view
    pub async fn project_structure<P: AsRef<Path>>(
        &self,
        path: P,
        max_depth: Option<u8>,
    ) -> DevItResult<ProjectStructure> {
        let path = path.as_ref();
        let user_path = self.adjust_user_path(path);
        let max_depth = max_depth.unwrap_or(MAX_TREE_DEPTH);

        // Validate path security
        let validated_path = self.path_security.validate_patch_path(user_path.as_ref())?;

        if !validated_path.exists() {
            return Err(DevItError::io(
                Some(validated_path.clone()),
                "read project structure",
                std::io::Error::new(std::io::ErrorKind::NotFound, "Path not found"),
            ));
        }

        // Detect project type
        let project_type = self.detect_project_type(&validated_path);

        // Build tree structure
        let mut total_files = 0;
        let mut total_dirs = 0;

        let tree = self.build_tree_node(
            &validated_path,
            0,
            max_depth,
            &mut total_files,
            &mut total_dirs,
        )?;

        Ok(ProjectStructure {
            root: user_path.as_ref().to_path_buf(),
            project_type,
            tree,
            total_files,
            total_dirs,
        })
    }

    /// Detect project type based on files present
    fn detect_project_type(&self, path: &Path) -> Option<String> {
        // Check for common project files
        let project_indicators = [
            ("Cargo.toml", "Rust"),
            ("package.json", "Node.js"),
            ("pom.xml", "Maven/Java"),
            ("build.gradle", "Gradle/Java"),
            ("pyproject.toml", "Python"),
            ("requirements.txt", "Python"),
            ("Gemfile", "Ruby"),
            ("composer.json", "PHP"),
            ("go.mod", "Go"),
            ("CMakeLists.txt", "C/C++"),
            ("Makefile", "C/C++/Make"),
            ("sln", "C#/.NET"),
            ("pubspec.yaml", "Dart/Flutter"),
        ];

        for (file, project_type) in project_indicators {
            if path.join(file).exists() {
                return Some(project_type.to_string());
            }
        }

        None
    }

    /// Build tree node recursively
    fn build_tree_node(
        &self,
        path: &Path,
        current_depth: u8,
        max_depth: u8,
        total_files: &mut usize,
        total_dirs: &mut usize,
    ) -> DevItResult<TreeNode> {
        let name = path
            .file_name()
            .unwrap_or_else(|| path.as_os_str())
            .to_string_lossy()
            .to_string();

        let metadata = fs::metadata(path)
            .map_err(|e| DevItError::io(Some(path.to_path_buf()), "read tree node metadata", e))?;

        let node_type = if metadata.is_dir() {
            *total_dirs += 1;
            FileType::Directory
        } else if metadata.is_symlink() {
            FileType::Symlink
        } else {
            *total_files += 1;
            FileType::File
        };

        let size = if metadata.is_file() {
            Some(metadata.len())
        } else {
            None
        };

        let mut children = None;

        // Recursively process children if it's a directory and we haven't reached max depth
        if metadata.is_dir() && current_depth < max_depth {
            let mut child_nodes = Vec::new();

            if let Ok(read_dir) = fs::read_dir(path) {
                let mut entries: Vec<_> = read_dir
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| self.should_include_path(&entry.path()))
                    .collect();

                // Sort entries: directories first, then files, alphabetically
                entries.sort_by(|a, b| {
                    let a_is_dir = a.path().is_dir();
                    let b_is_dir = b.path().is_dir();

                    if a_is_dir && !b_is_dir {
                        std::cmp::Ordering::Less
                    } else if !a_is_dir && b_is_dir {
                        std::cmp::Ordering::Greater
                    } else {
                        a.file_name().cmp(&b.file_name())
                    }
                });

                for entry in entries {
                    let entry_path = entry.path();
                    if let Ok(child_node) = self.build_tree_node(
                        &entry_path,
                        current_depth + 1,
                        max_depth,
                        total_files,
                        total_dirs,
                    ) {
                        child_nodes.push(child_node);
                    }
                }
            }

            if !child_nodes.is_empty() {
                children = Some(child_nodes);
            }
        }

        Ok(TreeNode {
            name,
            path: path.to_path_buf(),
            node_type,
            children,
            size,
        })
    }

    /// Validate if path should be included (respect .gitignore, etc.)
    fn should_include_path(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        // Skip common ignored directories
        if path_str.contains("/.git/")
            || path_str.contains("/target/")
            || path_str.contains("/node_modules/")
            || path_str.contains("/.devit/")
        {
            return false;
        }

        // Skip hidden files and directories (starting with .)
        if let Some(name) = path.file_name() {
            if name.to_string_lossy().starts_with('.') && name != ".gitignore" && name != ".gitkeep"
            {
                return false;
            }
        }

        true
    }
}

// ============================================================================
// Compressible Implementations for MCP Response Optimization
// ============================================================================

impl Compressible for FileEntry {
    fn to_format(&self, format: &OutputFormat) -> DevItResult<String> {
        match format {
            OutputFormat::Json => serde_json::to_string(self).map_err(|e| DevItError::Internal {
                component: "file_ops".to_string(),
                message: format!("JSON serialization failed: {}", e),
                cause: Some(e.to_string()),
                correlation_id: uuid::Uuid::new_v4().to_string(),
            }),
            OutputFormat::Compact => {
                let json = serde_json::to_string(self).map_err(|e| DevItError::Internal {
                    component: "file_ops".to_string(),
                    message: format!("JSON serialization failed: {}", e),
                    cause: Some(e.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;
                FieldMappings::apply_mappings(&json)
            }
            OutputFormat::Table => {
                let headers = ["name", "path", "type", "size", "permissions"];
                let json_value = serde_json::to_value(self).map_err(|e| DevItError::Internal {
                    component: "file_ops".to_string(),
                    message: format!("Value serialization failed: {}", e),
                    cause: Some(e.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;

                // Simplify permissions for table format
                let mut simplified = json_value.as_object().unwrap().clone();
                if let Some(perms) = simplified.get("permissions") {
                    let p = perms.as_object().unwrap();
                    let perm_str = format!(
                        "{}{}{}",
                        if p["readable"].as_bool().unwrap_or(false) {
                            "r"
                        } else {
                            "-"
                        },
                        if p["writable"].as_bool().unwrap_or(false) {
                            "w"
                        } else {
                            "-"
                        },
                        if p["executable"].as_bool().unwrap_or(false) {
                            "x"
                        } else {
                            "-"
                        }
                    );
                    simplified.insert(
                        "permissions".to_string(),
                        serde_json::Value::String(perm_str),
                    );
                }

                // Simplify entry_type
                if let Some(et) = simplified.get("entry_type") {
                    let simplified_type = match et.as_str().unwrap_or("file") {
                        "directory" => "dir",
                        "symlink" => "sym",
                        _ => "file",
                    };
                    simplified.insert(
                        "type".to_string(),
                        serde_json::Value::String(simplified_type.to_string()),
                    );
                    simplified.remove("entry_type");
                }

                FormatUtils::json_to_table_format(&serde_json::Value::Object(simplified), &headers)
            }
            OutputFormat::MessagePack => Err(DevItError::InvalidFormat {
                format: "messagepack".to_string(),
                supported: vec![
                    "json".to_string(),
                    "compact".to_string(),
                    "table".to_string(),
                ],
            }),
        }
    }

    fn get_compression_ratio(&self, format: &OutputFormat) -> DevItResult<f32> {
        let json_output = self.to_format(&OutputFormat::Json)?;
        let format_output = self.to_format(format)?;
        Ok(FormatUtils::calculate_compression_ratio(
            &json_output,
            &format_output,
        ))
    }

    fn get_field_mappings() -> std::collections::HashMap<String, String> {
        FieldMappings::get_mapping()
    }

    fn get_available_fields() -> Vec<String> {
        vec![
            "name".to_string(),
            "path".to_string(),
            "entry_type".to_string(),
            "size".to_string(),
            "modified".to_string(),
            "permissions".to_string(),
        ]
    }
}

impl Compressible for Vec<FileEntry> {
    fn to_format(&self, format: &OutputFormat) -> DevItResult<String> {
        match format {
            OutputFormat::Json => serde_json::to_string(self).map_err(|e| DevItError::Internal {
                component: "file_ops".to_string(),
                message: format!("JSON serialization failed: {}", e),
                cause: Some(e.to_string()),
                correlation_id: uuid::Uuid::new_v4().to_string(),
            }),
            OutputFormat::Compact => {
                let json = serde_json::to_string(self).map_err(|e| DevItError::Internal {
                    component: "file_ops".to_string(),
                    message: format!("JSON serialization failed: {}", e),
                    cause: Some(e.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;
                FieldMappings::apply_mappings(&json)
            }
            OutputFormat::Table => {
                if self.is_empty() {
                    return Ok("name|path|type|size|permissions\n".to_string());
                }

                let mut result = String::from("name|path|type|size|permissions\n");
                for entry in self {
                    let table_output = entry.to_format(&OutputFormat::Table)?;
                    // Skip header line from individual entry
                    let lines: Vec<&str> = table_output.lines().collect();
                    if lines.len() > 1 {
                        result.push_str(lines[1]);
                        result.push('\n');
                    }
                }
                Ok(result)
            }
            OutputFormat::MessagePack => Err(DevItError::InvalidFormat {
                format: "messagepack".to_string(),
                supported: vec![
                    "json".to_string(),
                    "compact".to_string(),
                    "table".to_string(),
                ],
            }),
        }
    }

    fn get_compression_ratio(&self, format: &OutputFormat) -> DevItResult<f32> {
        let json_output = self.to_format(&OutputFormat::Json)?;
        let format_output = self.to_format(format)?;
        Ok(FormatUtils::calculate_compression_ratio(
            &json_output,
            &format_output,
        ))
    }

    fn get_field_mappings() -> std::collections::HashMap<String, String> {
        FieldMappings::get_mapping()
    }

    fn get_available_fields() -> Vec<String> {
        FileEntry::get_available_fields()
    }
}

impl Compressible for FileContent {
    fn to_format(&self, format: &OutputFormat) -> DevItResult<String> {
        match format {
            OutputFormat::Json => serde_json::to_string(self).map_err(|e| DevItError::Internal {
                component: "file_ops".to_string(),
                message: format!("JSON serialization failed: {}", e),
                cause: Some(e.to_string()),
                correlation_id: uuid::Uuid::new_v4().to_string(),
            }),
            OutputFormat::Compact => {
                let json = serde_json::to_string(self).map_err(|e| DevItError::Internal {
                    component: "file_ops".to_string(),
                    message: format!("JSON serialization failed: {}", e),
                    cause: Some(e.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;
                FieldMappings::apply_mappings(&json)
            }
            OutputFormat::Table => {
                let headers = ["path", "size", "encoding", "content"];
                let json_value = serde_json::to_value(self).map_err(|e| DevItError::Internal {
                    component: "file_ops".to_string(),
                    message: format!("Value serialization failed: {}", e),
                    cause: Some(e.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;

                // Truncate content for table format to avoid huge rows
                let mut simplified = json_value.as_object().unwrap().clone();
                if let Some(content) = simplified.get("content") {
                    if let Some(content_str) = content.as_str() {
                        let truncated = if content_str.len() > 100 {
                            format!("{}...", &content_str[..97])
                        } else {
                            content_str.to_string()
                        };
                        simplified
                            .insert("content".to_string(), serde_json::Value::String(truncated));
                    }
                }

                FormatUtils::json_to_table_format(&serde_json::Value::Object(simplified), &headers)
            }
            OutputFormat::MessagePack => Err(DevItError::InvalidFormat {
                format: "messagepack".to_string(),
                supported: vec![
                    "json".to_string(),
                    "compact".to_string(),
                    "table".to_string(),
                ],
            }),
        }
    }

    fn get_compression_ratio(&self, format: &OutputFormat) -> DevItResult<f32> {
        let json_output = self.to_format(&OutputFormat::Json)?;
        let format_output = self.to_format(format)?;
        Ok(FormatUtils::calculate_compression_ratio(
            &json_output,
            &format_output,
        ))
    }

    fn get_field_mappings() -> std::collections::HashMap<String, String> {
        FieldMappings::get_mapping()
    }

    fn get_available_fields() -> Vec<String> {
        vec![
            "path".to_string(),
            "content".to_string(),
            "size".to_string(),
            "lines".to_string(),
            "encoding".to_string(),
        ]
    }
}

impl Compressible for SearchResults {
    fn to_format(&self, format: &OutputFormat) -> DevItResult<String> {
        match format {
            OutputFormat::Json => serde_json::to_string(self).map_err(|e| DevItError::Internal {
                component: "file_ops".to_string(),
                message: format!("JSON serialization failed: {}", e),
                cause: Some(e.to_string()),
                correlation_id: uuid::Uuid::new_v4().to_string(),
            }),
            OutputFormat::Compact => {
                let json = serde_json::to_string(self).map_err(|e| DevItError::Internal {
                    component: "file_ops".to_string(),
                    message: format!("JSON serialization failed: {}", e),
                    cause: Some(e.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;
                FieldMappings::apply_mappings(&json)
            }
            OutputFormat::Table => {
                let mut result = String::from("file|line|match|context\n");
                for search_match in &self.matches {
                    let context = format!(
                        "{}|{}",
                        search_match.context_before.join(" "),
                        search_match.context_after.join(" ")
                    );
                    result.push_str(&format!(
                        "{}|{}|{}|{}\n",
                        search_match.file.display().to_string().replace('|', "\\|"),
                        search_match.line_number,
                        search_match.line.replace('|', "\\|"),
                        context.replace('|', "\\|")
                    ));
                }
                Ok(result)
            }
            OutputFormat::MessagePack => Err(DevItError::InvalidFormat {
                format: "messagepack".to_string(),
                supported: vec![
                    "json".to_string(),
                    "compact".to_string(),
                    "table".to_string(),
                ],
            }),
        }
    }

    fn get_compression_ratio(&self, format: &OutputFormat) -> DevItResult<f32> {
        let json_output = self.to_format(&OutputFormat::Json)?;
        let format_output = self.to_format(format)?;
        Ok(FormatUtils::calculate_compression_ratio(
            &json_output,
            &format_output,
        ))
    }

    fn get_field_mappings() -> std::collections::HashMap<String, String> {
        FieldMappings::get_mapping()
    }

    fn get_available_fields() -> Vec<String> {
        vec![
            "pattern".to_string(),
            "path".to_string(),
            "files_searched".to_string(),
            "total_matches".to_string(),
            "matches".to_string(),
            "truncated".to_string(),
        ]
    }
}

// Implement Compressible for ProjectStructure
impl Compressible for ProjectStructure {
    fn to_format(&self, format: &OutputFormat) -> DevItResult<String> {
        match format {
            OutputFormat::Json => serde_json::to_string(self).map_err(|e| DevItError::Internal {
                component: "file_ops".to_string(),
                message: format!("JSON serialization failed: {}", e),
                cause: Some(e.to_string()),
                correlation_id: uuid::Uuid::new_v4().to_string(),
            }),
            OutputFormat::Compact => {
                let json = serde_json::to_string(self).map_err(|e| DevItError::Internal {
                    component: "file_ops".to_string(),
                    message: format!("JSON serialization failed: {}", e),
                    cause: Some(e.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;
                FieldMappings::apply_mappings(&json)
            }
            OutputFormat::Table => {
                // Generate a tabular representation of the project structure
                let mut result = String::from("name|type|path|level\n");

                fn traverse_tree(
                    node: &TreeNode,
                    level: usize,
                    result: &mut String,
                    base_path: &std::path::Path,
                ) {
                    let path = if level == 0 {
                        base_path.display().to_string()
                    } else {
                        base_path.join(&node.name).display().to_string()
                    };

                    let node_type = match node.node_type {
                        FileType::File => "File",
                        FileType::Directory => "Directory",
                        FileType::Symlink => "Symlink",
                    };

                    result.push_str(&format!(
                        "{}|{}|{}|{}\n",
                        node.name.replace('|', "\\|"),
                        node_type,
                        path.replace('|', "\\|"),
                        level
                    ));

                    if let Some(children) = &node.children {
                        for child in children {
                            traverse_tree(child, level + 1, result, base_path);
                        }
                    }
                }

                traverse_tree(&self.tree, 0, &mut result, &self.root);
                Ok(result)
            }
            OutputFormat::MessagePack => Err(DevItError::InvalidFormat {
                format: "messagepack".to_string(),
                supported: vec![
                    "json".to_string(),
                    "compact".to_string(),
                    "table".to_string(),
                ],
            }),
        }
    }

    fn get_compression_ratio(&self, format: &OutputFormat) -> DevItResult<f32> {
        let json_output = self.to_format(&OutputFormat::Json)?;
        let format_output = self.to_format(format)?;
        Ok(FormatUtils::calculate_compression_ratio(
            &json_output,
            &format_output,
        ))
    }

    fn get_field_mappings() -> std::collections::HashMap<String, String> {
        FieldMappings::get_mapping()
    }

    fn get_available_fields() -> Vec<String> {
        vec![
            "root".to_string(),
            "project_type".to_string(),
            "tree".to_string(),
            "total_files".to_string(),
            "total_dirs".to_string(),
        ]
    }
}
