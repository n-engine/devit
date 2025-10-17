use crate::core::errors::{DevItError, DevItResult};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PatchHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<PatchLine>,
}

#[derive(Debug, Clone)]
pub enum PatchLine {
    Context(String),
    Add(String),
    Remove(String),
}

#[derive(Debug)]
pub struct FilePatch {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub hunks: Vec<PatchHunk>,
    pub is_new_file: bool,
    pub is_deleted_file: bool,
    pub old_mode: Option<u32>,
    pub new_mode: Option<u32>,
    pub adds_exec_bit: bool,
    pub is_binary: bool,
}

#[derive(Debug)]
pub struct ParsedPatch {
    pub files: Vec<FilePatch>,
}

impl ParsedPatch {
    pub fn from_diff(diff_content: &str) -> DevItResult<Self> {
        let mut files = Vec::new();
        let lines: Vec<&str> = diff_content.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            if lines[i].starts_with("diff --git ") {
                let (file_patch, next_index) = Self::parse_file_patch(&lines, i)?;
                files.push(file_patch);
                i = next_index;
            } else {
                i += 1;
            }
        }

        Ok(ParsedPatch { files })
    }

    fn parse_file_patch(lines: &[&str], start: usize) -> DevItResult<(FilePatch, usize)> {
        let mut i = start;
        let mut old_path = None;
        let mut new_path = None;
        let mut is_new_file = false;
        let mut is_deleted_file = false;
        let mut old_mode = None;
        let mut new_mode = None;
        let mut hunks = Vec::new();
        let mut is_binary = false;

        // Parse diff header
        while i < lines.len() && !lines[i].starts_with("@@") {
            if let Some(rest) = lines[i].strip_prefix("old mode ") {
                old_mode = parse_mode(rest.trim(), i + 1)?;
            } else if let Some(rest) = lines[i].strip_prefix("new mode ") {
                new_mode = parse_mode(rest.trim(), i + 1)?;
            } else if lines[i].starts_with("--- ") {
                let path_str = &lines[i][4..];
                if path_str != "/dev/null" {
                    old_path = Some(PathBuf::from(path_str.trim_start_matches("a/")));
                }
            } else if lines[i].starts_with("+++ ") {
                let path_str = &lines[i][4..];
                if path_str != "/dev/null" {
                    new_path = Some(PathBuf::from(path_str.trim_start_matches("b/")));
                }
            } else if lines[i].contains("new file mode") {
                is_new_file = true;
            } else if lines[i].contains("deleted file mode") {
                is_deleted_file = true;
            } else if lines[i].starts_with("Binary files ") {
                is_binary = true;
                i += 1;
                break;
            }
            i += 1;
        }

        // Parse hunks
        while i < lines.len() && lines[i].starts_with("@@") {
            let (hunk, next_index) = Self::parse_hunk(lines, i)?;
            hunks.push(hunk);
            i = next_index;
        }

        let file_patch = FilePatch {
            old_path,
            new_path,
            hunks,
            is_new_file,
            is_deleted_file,
            old_mode,
            new_mode,
            adds_exec_bit: mode_adds_exec(old_mode, new_mode),
            is_binary,
        };

        Ok((file_patch, i))
    }

    fn parse_hunk(lines: &[&str], start: usize) -> DevItResult<(PatchHunk, usize)> {
        let hunk_header = lines[start];

        // Parse @@ -old_start,old_count +new_start,new_count @@
        let parts: Vec<&str> = hunk_header.split_whitespace().collect();
        if parts.len() < 3 {
            return Err(DevItError::InvalidDiff {
                reason: format!("Invalid hunk header: {}", hunk_header),
                line_number: Some(start + 1),
            });
        }

        let old_range = &parts[1][1..]; // Remove '-'
        let new_range = &parts[2][1..]; // Remove '+'

        let (old_start, old_count) = Self::parse_range(old_range)?;
        let (new_start, new_count) = Self::parse_range(new_range)?;

        let mut hunk_lines = Vec::new();
        let mut i = start + 1;

        while i < lines.len() {
            let line = lines[i];
            if line.starts_with("@@") || line.starts_with("diff --git") {
                break;
            }

            match line.chars().next() {
                Some(' ') => hunk_lines.push(PatchLine::Context(line[1..].to_string())),
                Some('+') => hunk_lines.push(PatchLine::Add(line[1..].to_string())),
                Some('-') => hunk_lines.push(PatchLine::Remove(line[1..].to_string())),
                _ => break, // End of hunk
            }
            i += 1;
        }

        let hunk = PatchHunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines: hunk_lines,
        };

        Ok((hunk, i))
    }

    fn parse_range(range: &str) -> DevItResult<(usize, usize)> {
        if let Some(comma_pos) = range.find(',') {
            let start = range[..comma_pos]
                .parse()
                .map_err(|_| DevItError::InvalidDiff {
                    reason: format!("Invalid range start: {}", range),
                    line_number: None,
                })?;
            let count = range[comma_pos + 1..]
                .parse()
                .map_err(|_| DevItError::InvalidDiff {
                    reason: format!("Invalid range count: {}", range),
                    line_number: None,
                })?;
            Ok((start, count))
        } else {
            let start = range.parse().map_err(|_| DevItError::InvalidDiff {
                reason: format!("Invalid range: {}", range),
                line_number: None,
            })?;
            Ok((start, 1))
        }
    }
}

fn parse_mode(value: &str, line_number: usize) -> DevItResult<Option<u32>> {
    if value.is_empty() {
        return Ok(None);
    }
    u32::from_str_radix(value, 8)
        .map(Some)
        .map_err(|_| DevItError::InvalidDiff {
            reason: format!("Invalid file mode '{}'", value),
            line_number: Some(line_number),
        })
}

fn mode_adds_exec(old_mode: Option<u32>, new_mode: Option<u32>) -> bool {
    const EXEC_MASK: u32 = 0o111;
    match (old_mode, new_mode) {
        (Some(old), Some(new)) => (new & EXEC_MASK) != 0 && (old & EXEC_MASK) == 0,
        (None, Some(new)) => (new & EXEC_MASK) != 0,
        _ => false,
    }
}
