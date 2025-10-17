use crate::core::errors::{DevItError, DevItResult};
use crate::core::patch_parser::{FilePatch, ParsedPatch, PatchHunk, PatchLine};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct AtomicPatcher {
    working_dir: PathBuf,
    dry_run: bool,
}

pub struct PatchStats {
    pub files_modified: usize,
    pub hunks_applied: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_created: usize,
    pub files_deleted: usize,
}

impl AtomicPatcher {
    pub fn new(working_dir: PathBuf, dry_run: bool) -> Self {
        Self {
            working_dir,
            dry_run,
        }
    }

    pub fn apply_patch(&self, patch_content: &str) -> DevItResult<PatchStats> {
        let parsed = ParsedPatch::from_diff(patch_content)?;
        let mut stats = PatchStats {
            files_modified: 0,
            hunks_applied: 0,
            lines_added: 0,
            lines_removed: 0,
            files_created: 0,
            files_deleted: 0,
        };

        // Security validation
        self.validate_security(&parsed)?;

        // Apply each file patch
        for file_patch in &parsed.files {
            self.apply_file_patch(file_patch, &mut stats)?;
        }

        Ok(stats)
    }

    fn validate_security(&self, parsed: &ParsedPatch) -> DevItResult<()> {
        for file_patch in &parsed.files {
            // Check all relevant paths
            let paths = [&file_patch.old_path, &file_patch.new_path];

            for path_opt in &paths {
                if let Some(path) = path_opt {
                    self.validate_path(path)?;
                }
            }
        }
        Ok(())
    }

    fn validate_path(&self, path: &Path) -> DevItResult<()> {
        // Reject absolute paths
        if path.is_absolute() {
            return Err(DevItError::ProtectedPath {
                path: path.to_path_buf(),
                protection_rule: "no_absolute_paths".to_string(),
                attempted_operation: "patch_apply".to_string(),
            });
        }

        // Reject path traversal attempts
        for component in path.components() {
            if let std::path::Component::ParentDir = component {
                return Err(DevItError::ProtectedPath {
                    path: path.to_path_buf(),
                    protection_rule: "no_path_traversal".to_string(),
                    attempted_operation: "patch_apply".to_string(),
                });
            }
        }

        // Check for symlinks in the actual path if it exists
        let full_path = self.working_dir.join(path);
        if full_path.exists() {
            let metadata = std::fs::symlink_metadata(&full_path)
                .map_err(|e| DevItError::io(Some(full_path.clone()), "symlink_metadata", e))?;

            if metadata.file_type().is_symlink() {
                return Err(DevItError::ProtectedPath {
                    path: path.to_path_buf(),
                    protection_rule: "no_symlinks".to_string(),
                    attempted_operation: "patch_apply".to_string(),
                });
            }
        }

        Ok(())
    }

    fn apply_file_patch(&self, file_patch: &FilePatch, stats: &mut PatchStats) -> DevItResult<()> {
        if file_patch.is_deleted_file {
            self.delete_file(file_patch, stats)?;
        } else if file_patch.is_new_file {
            self.create_file(file_patch, stats)?;
        } else {
            self.modify_file(file_patch, stats)?;
        }
        Ok(())
    }

    fn delete_file(&self, file_patch: &FilePatch, stats: &mut PatchStats) -> DevItResult<()> {
        let path = file_patch
            .old_path
            .as_ref()
            .ok_or_else(|| DevItError::InvalidDiff {
                reason: "Deleted file missing old path".to_string(),
                line_number: None,
            })?;

        let full_path = self.working_dir.join(path);

        if !self.dry_run {
            if full_path.exists() {
                std::fs::remove_file(&full_path)
                    .map_err(|e| DevItError::io(Some(full_path), "delete file", e))?;
            }
        }

        stats.files_deleted += 1;
        Ok(())
    }

    fn create_file(&self, file_patch: &FilePatch, stats: &mut PatchStats) -> DevItResult<()> {
        let path = file_patch
            .new_path
            .as_ref()
            .ok_or_else(|| DevItError::InvalidDiff {
                reason: "New file missing new path".to_string(),
                line_number: None,
            })?;

        let full_path = self.working_dir.join(path);

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            if !self.dry_run {
                std::fs::create_dir_all(parent).map_err(|e| {
                    DevItError::io(Some(parent.to_path_buf()), "create parent directories", e)
                })?;
            }
        }

        // Build content from hunks
        let content = self.build_new_content(&file_patch.hunks, &[])?;

        if !self.dry_run {
            self.write_file_atomically(&full_path, &content)?;
        }

        stats.files_created += 1;
        self.update_stats_from_hunks(&file_patch.hunks, stats);
        Ok(())
    }

    fn modify_file(&self, file_patch: &FilePatch, stats: &mut PatchStats) -> DevItResult<()> {
        let path = file_patch
            .new_path
            .as_ref()
            .or(file_patch.old_path.as_ref())
            .ok_or_else(|| DevItError::InvalidDiff {
                reason: "Modified file missing path".to_string(),
                line_number: None,
            })?;

        let full_path = self.working_dir.join(path);

        // Read existing file
        let original_lines = if full_path.exists() {
            self.read_file_lines(&full_path)?
        } else {
            Vec::new()
        };

        // Apply hunks and build new content
        let new_content = self.build_new_content(&file_patch.hunks, &original_lines)?;

        if !self.dry_run {
            self.write_file_atomically(&full_path, &new_content)?;
        }

        stats.files_modified += 1;
        self.update_stats_from_hunks(&file_patch.hunks, stats);
        Ok(())
    }

    fn read_file_lines(&self, path: &Path) -> DevItResult<Vec<String>> {
        let file = File::open(path)
            .map_err(|e| DevItError::io(Some(path.to_path_buf()), "open file for reading", e))?;

        let reader = BufReader::new(file);
        let lines: Result<Vec<String>, _> = reader.lines().collect();

        lines.map_err(|e| DevItError::io(Some(path.to_path_buf()), "read file lines", e))
    }

    fn build_new_content(
        &self,
        hunks: &[PatchHunk],
        original_lines: &[String],
    ) -> DevItResult<String> {
        let mut result_lines = original_lines.to_vec();

        // Apply hunks in reverse order to maintain line numbers
        for hunk in hunks.iter().rev() {
            self.apply_hunk_to_lines(&mut result_lines, hunk)?;
        }

        Ok(result_lines.join("\n"))
    }

    fn apply_hunk_to_lines(&self, lines: &mut Vec<String>, hunk: &PatchHunk) -> DevItResult<()> {
        let start_idx = if hunk.old_start > 0 {
            hunk.old_start - 1
        } else {
            0
        };
        if start_idx > lines.len() {
            return Err(DevItError::InvalidDiff {
                reason: format!(
                    "Patch context starts at line {} but file has only {} lines",
                    hunk.old_start,
                    lines.len()
                ),
                line_number: Some(hunk.old_start),
            });
        }
        let mut old_idx = start_idx;
        let mut patch_idx = 0;

        // Validate context lines before applying
        while patch_idx < hunk.lines.len() {
            match &hunk.lines[patch_idx] {
                PatchLine::Context(context_line) => {
                    if old_idx < lines.len() && &lines[old_idx] != context_line {
                        return Err(DevItError::VcsConflict {
                            location: format!("line {}", old_idx + 1),
                            conflict_type: "context_mismatch".to_string(),
                            conflicted_files: vec![],
                            resolution_hint: Some(format!(
                                "Expected: '{}', Found: '{}'",
                                context_line,
                                lines.get(old_idx).unwrap_or(&String::new())
                            )),
                        });
                    }
                    old_idx += 1;
                    patch_idx += 1;
                }
                PatchLine::Remove(remove_line) => {
                    if old_idx < lines.len() && &lines[old_idx] != remove_line {
                        return Err(DevItError::VcsConflict {
                            location: format!("line {}", old_idx + 1),
                            conflict_type: "remove_mismatch".to_string(),
                            conflicted_files: vec![],
                            resolution_hint: Some(format!(
                                "Expected to remove: '{}', Found: '{}'",
                                remove_line,
                                lines.get(old_idx).unwrap_or(&String::new())
                            )),
                        });
                    }
                    old_idx += 1;
                    patch_idx += 1;
                }
                PatchLine::Add(_) => {
                    patch_idx += 1;
                }
            }
        }

        // Now apply the changes
        old_idx = start_idx;
        patch_idx = 0;
        let mut new_lines = Vec::new();

        // Copy lines before the hunk
        let prefix_end = start_idx.min(lines.len());
        new_lines.extend_from_slice(&lines[..prefix_end]);

        // Apply hunk changes
        while patch_idx < hunk.lines.len() {
            match &hunk.lines[patch_idx] {
                PatchLine::Context(line) => {
                    new_lines.push(line.clone());
                    old_idx += 1;
                    patch_idx += 1;
                }
                PatchLine::Remove(_) => {
                    // Skip the removed line
                    old_idx += 1;
                    patch_idx += 1;
                }
                PatchLine::Add(line) => {
                    new_lines.push(line.clone());
                    patch_idx += 1;
                }
            }
        }

        // Copy remaining lines
        if old_idx > lines.len() {
            return Err(DevItError::InvalidDiff {
                reason: format!(
                    "Patch consumes more lines than available (line {} in file with {} lines)",
                    old_idx + 1,
                    lines.len()
                ),
                line_number: Some(old_idx + 1),
            });
        }
        new_lines.extend_from_slice(&lines[old_idx..]);

        *lines = new_lines;
        Ok(())
    }

    fn write_file_atomically(&self, path: &Path, content: &str) -> DevItResult<()> {
        let temp_path = path.with_extension("devit.tmp");

        // Write to temp file
        let mut temp_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)
            .map_err(|e| DevItError::io(Some(temp_path.clone()), "create temp file", e))?;

        temp_file
            .write_all(content.as_bytes())
            .map_err(|e| DevItError::io(Some(temp_path.clone()), "write to temp file", e))?;

        // Sync to disk
        temp_file
            .sync_all()
            .map_err(|e| DevItError::io(Some(temp_path.clone()), "sync temp file", e))?;

        drop(temp_file);

        // Atomic rename
        std::fs::rename(&temp_path, path)
            .map_err(|e| DevItError::io(Some(path.to_path_buf()), "atomic rename", e))?;

        Ok(())
    }

    fn update_stats_from_hunks(&self, hunks: &[PatchHunk], stats: &mut PatchStats) {
        stats.hunks_applied += hunks.len();

        for hunk in hunks {
            for line in &hunk.lines {
                match line {
                    PatchLine::Add(_) => stats.lines_added += 1,
                    PatchLine::Remove(_) => stats.lines_removed += 1,
                    PatchLine::Context(_) => {} // Context lines don't count
                }
            }
        }
    }
}
