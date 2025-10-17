use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use tokio::sync::RwLock;

use crate::core::{
    errors::{DevItError, DevItResult},
    file_ops,
    formats::{self, Compressible},
};

/// Centralise toutes les opérations fichiers/hierarchie pour réutilisation multi-binaires.
pub struct FsService {
    manager: RwLock<file_ops::FileOpsManager>,
    root_path: PathBuf,
}

impl FsService {
    /// Crée un service fichiers attaché à un répertoire racine.
    pub fn new(root: PathBuf) -> DevItResult<Self> {
        let manager = file_ops::FileOpsManager::new(root)?;
        let root_path = manager.get_root_path().to_path_buf();
        Ok(Self {
            manager: RwLock::new(manager),
            root_path,
        })
    }

    /// Lecture de fichier avec numéros de lignes optionnels.
    pub async fn read<P: AsRef<Path>>(
        &self,
        path: P,
        line_numbers: bool,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> DevItResult<file_ops::FileContent> {
        let manager = self.manager.read().await;
        manager.file_read(path, line_numbers, offset, limit).await
    }

    /// Liste de fichiers/dossiers avec métadonnées.
    pub async fn list<P: AsRef<Path>>(
        &self,
        path: P,
        recursive: bool,
    ) -> DevItResult<Vec<file_ops::FileEntry>> {
        let manager = self.manager.read().await;
        manager.file_list(path, recursive).await
    }

    /// Recherche de motif dans les fichiers.
    pub async fn search<P: AsRef<Path>>(
        &self,
        pattern: &str,
        path: P,
        context_lines: Option<usize>,
    ) -> DevItResult<file_ops::SearchResults> {
        let manager = self.manager.read().await;
        manager.file_search(pattern, path, context_lines).await
    }

    /// Structure projet (arbre).
    pub async fn project_structure<P: AsRef<Path>>(
        &self,
        path: P,
        max_depth: Option<u8>,
    ) -> DevItResult<file_ops::ProjectStructure> {
        let manager = self.manager.read().await;
        manager.project_structure(path, max_depth).await
    }

    /// Racine de travail détectée.
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Lecture avec compression et filtrage.
    pub async fn read_ext<P: AsRef<Path>>(
        &self,
        path: P,
        format: &formats::OutputFormat,
        fields: Option<&[String]>,
        line_numbers: Option<bool>,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> DevItResult<String> {
        let line_numbers = line_numbers.unwrap_or(false);
        let content = self
            .read(
                path,
                line_numbers,
                offset.map(|o| o as usize),
                limit.map(|l| l as usize),
            )
            .await?;

        let filtered = if let Some(field_list) = fields {
            self.filter_file_content_fields(&content, field_list, line_numbers)?
        } else {
            content
        };

        filtered.to_format(format)
    }

    /// Listing avec options étendues.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_ext<P: AsRef<Path>>(
        &self,
        path: P,
        format: &formats::OutputFormat,
        fields: Option<&[String]>,
        recursive: Option<bool>,
        include_hidden: Option<bool>,
        include_patterns: Option<&[String]>,
        exclude_patterns: Option<&[String]>,
    ) -> DevItResult<String> {
        let recursive = recursive.unwrap_or(false);
        let include_hidden = include_hidden.unwrap_or(false);
        let include_filter = Self::build_globset(include_patterns, "include_patterns")?;
        let exclude_filter = Self::build_globset(exclude_patterns, "exclude_patterns")?;

        let entries = self.list(path, recursive).await?;

        let entries: Vec<_> = entries
            .into_iter()
            .filter(|entry| include_hidden || !Self::is_hidden(&entry.path))
            .filter(|entry| {
                include_filter
                    .as_ref()
                    .map(|set| Self::glob_matches(&entry.path, set))
                    .unwrap_or(true)
            })
            .filter(|entry| {
                exclude_filter
                    .as_ref()
                    .map(|set| !Self::glob_matches(&entry.path, set))
                    .unwrap_or(true)
            })
            .collect();

        let filtered = if let Some(field_list) = fields {
            self.filter_file_list_fields(&entries, field_list)?
        } else {
            entries
        };

        filtered.to_format(format)
    }

    /// Recherche avec options étendues.
    pub async fn search_ext<P: AsRef<Path>>(
        &self,
        pattern: &str,
        path: P,
        format: &formats::OutputFormat,
        fields: Option<&[String]>,
        context_lines: Option<u8>,
        file_pattern: Option<&str>,
        max_results: Option<usize>,
    ) -> DevItResult<String> {
        let _ = (file_pattern, max_results);
        let results = self
            .search(pattern, path, context_lines.map(|cl| cl as usize))
            .await?;

        let filtered = if let Some(field_list) = fields {
            self.filter_search_results_fields(&results, field_list)?
        } else {
            results
        };

        filtered.to_format(format)
    }

    /// Structure projet avec compression.
    pub async fn project_structure_ext<P: AsRef<Path>>(
        &self,
        path: P,
        format: &formats::OutputFormat,
        fields: Option<&[String]>,
        max_depth: Option<u8>,
    ) -> DevItResult<String> {
        let structure = self.project_structure(path, max_depth).await?;
        let _ = fields; // Pour future sélection de champs
        structure.to_format(format)
    }

    fn is_hidden(path: &Path) -> bool {
        path.components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .map(|segment| segment.starts_with('.') && segment != "." && segment != "..")
                .unwrap_or(false)
        })
    }

    fn glob_matches(path: &Path, set: &GlobSet) -> bool {
        let candidate = path.to_string_lossy();
        set.is_match(candidate.as_ref())
    }

    fn build_globset(patterns: Option<&[String]>, label: &str) -> DevItResult<Option<GlobSet>> {
        let Some(patterns) = patterns else {
            return Ok(None);
        };

        if patterns.is_empty() {
            return Ok(None);
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = Glob::new(pattern).map_err(|err| DevItError::InvalidFormat {
                format: format!("{label}: '{pattern}' ({err})"),
                supported: vec!["glob pattern".into()],
            })?;
            builder.add(glob);
        }

        builder
            .build()
            .map(Some)
            .map_err(|err| DevItError::InvalidFormat {
                format: format!("{label}: {err}"),
                supported: vec!["glob pattern".into()],
            })
    }

    fn field_requested(fields: &[String], name: &str) -> bool {
        fields.iter().any(|f| f == name)
    }

    fn filter_file_content_fields(
        &self,
        content: &file_ops::FileContent,
        fields: &[String],
        include_line_numbers: bool,
    ) -> DevItResult<file_ops::FileContent> {
        for field in fields {
            match field.as_str() {
                "path" | "content" | "size" | "lines" | "encoding" => {}
                other => {
                    return Err(DevItError::InvalidFormat {
                        format: other.to_string(),
                        supported: vec![
                            "path".into(),
                            "content".into(),
                            "size".into(),
                            "lines".into(),
                            "encoding".into(),
                        ],
                    });
                }
            }
        }

        let mut filtered = file_ops::FileContent {
            path: if Self::field_requested(fields, "path") {
                content.path.clone()
            } else {
                PathBuf::new()
            },
            content: if Self::field_requested(fields, "content") {
                content.content.clone()
            } else {
                String::new()
            },
            size: if Self::field_requested(fields, "size") {
                content.size
            } else {
                0
            },
            lines: if include_line_numbers && Self::field_requested(fields, "lines") {
                content.lines.clone()
            } else {
                None
            },
            encoding: if Self::field_requested(fields, "encoding") {
                content.encoding.clone()
            } else {
                String::new()
            },
        };

        // Ensure required defaults when no fields were specified
        if fields.is_empty() {
            filtered.path = content.path.clone();
            filtered.content = content.content.clone();
            filtered.size = content.size;
            filtered.encoding = content.encoding.clone();
            if include_line_numbers {
                filtered.lines = content.lines.clone();
            }
        }

        Ok(filtered)
    }

    fn filter_file_list_fields(
        &self,
        list: &[file_ops::FileEntry],
        fields: &[String],
    ) -> DevItResult<Vec<file_ops::FileEntry>> {
        let mut filtered_list = Vec::with_capacity(list.len());

        for entry in list {
            let mut filtered_entry = file_ops::FileEntry {
                name: String::new(),
                path: PathBuf::new(),
                entry_type: file_ops::FileType::File,
                size: None,
                modified: None,
                permissions: file_ops::FilePermissions {
                    readable: false,
                    writable: false,
                    executable: false,
                },
            };

            for field in fields {
                match field.as_str() {
                    "name" => filtered_entry.name = entry.name.clone(),
                    "path" => filtered_entry.path = entry.path.clone(),
                    "type" | "entry_type" => {
                        filtered_entry.entry_type = entry.entry_type.clone();
                    }
                    "size" => filtered_entry.size = entry.size,
                    "modified" => filtered_entry.modified = entry.modified,
                    "permissions" => {
                        filtered_entry.permissions = entry.permissions.clone();
                    }
                    other => {
                        return Err(DevItError::InvalidFormat {
                            format: other.to_string(),
                            supported: vec![
                                "name".into(),
                                "path".into(),
                                "type".into(),
                                "size".into(),
                                "modified".into(),
                                "permissions".into(),
                            ],
                        });
                    }
                }
            }

            if !Self::field_requested(fields, "name") {
                filtered_entry.name = entry.name.clone();
            }
            if !Self::field_requested(fields, "path") {
                filtered_entry.path = entry.path.clone();
            }
            if !Self::field_requested(fields, "type")
                && !Self::field_requested(fields, "entry_type")
            {
                filtered_entry.entry_type = entry.entry_type.clone();
            }
            if !Self::field_requested(fields, "size") {
                filtered_entry.size = entry.size;
            }
            if !Self::field_requested(fields, "modified") {
                filtered_entry.modified = entry.modified;
            }
            if !Self::field_requested(fields, "permissions") {
                filtered_entry.permissions = entry.permissions.clone();
            }

            filtered_list.push(filtered_entry);
        }

        Ok(filtered_list)
    }

    fn filter_search_results_fields(
        &self,
        results: &file_ops::SearchResults,
        fields: &[String],
    ) -> DevItResult<file_ops::SearchResults> {
        let mut filtered = file_ops::SearchResults {
            pattern: String::new(),
            path: PathBuf::new(),
            files_searched: 0,
            total_matches: 0,
            matches: Vec::new(),
            truncated: false,
        };

        for field in fields {
            match field.as_str() {
                "pattern" => filtered.pattern = results.pattern.clone(),
                "path" => filtered.path = results.path.clone(),
                "files_searched" => filtered.files_searched = results.files_searched,
                "total_matches" => filtered.total_matches = results.total_matches,
                "matches" => filtered.matches = results.matches.clone(),
                "truncated" => filtered.truncated = results.truncated,
                other => {
                    return Err(DevItError::InvalidFormat {
                        format: other.to_string(),
                        supported: vec![
                            "pattern".into(),
                            "path".into(),
                            "files_searched".into(),
                            "total_matches".into(),
                            "matches".into(),
                            "truncated".into(),
                        ],
                    });
                }
            }
        }

        if !Self::field_requested(fields, "pattern") {
            filtered.pattern = results.pattern.clone();
        }
        if !Self::field_requested(fields, "path") {
            filtered.path = results.path.clone();
        }
        if !Self::field_requested(fields, "files_searched") {
            filtered.files_searched = results.files_searched;
        }
        if !Self::field_requested(fields, "total_matches") {
            filtered.total_matches = results.total_matches;
        }
        if !Self::field_requested(fields, "matches") {
            filtered.matches = results.matches.clone();
        }
        if !Self::field_requested(fields, "truncated") {
            filtered.truncated = results.truncated;
        }

        Ok(filtered)
    }
}
