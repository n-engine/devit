use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde_json::Value;
use walkdir::{DirEntry, WalkDir};

fn main() -> Result<()> {
    let repo_root = env::current_dir().context("unable to determine current directory")?;
    let run_build = env::var("DEVIT_TRACKING_RUN_BUILD")
        .map(|value| value != "0")
        .unwrap_or(true);
    let run_tests = env::var("DEVIT_TRACKING_RUN_TESTS")
        .map(|value| value != "0")
        .unwrap_or(false);
    let should_write = env::var("DEVIT_TRACKING_WRITE")
        .map(|value| value != "0")
        .unwrap_or(true);

    let timestamp = Utc::now();

    let feature_docs = collect_documents(
        repo_root.join("PROJECT_TRACKING/FEATURES"),
        CollectOptions::default(),
    )?;
    let bug_docs = collect_documents(
        repo_root.join("PROJECT_TRACKING/BUGS"),
        CollectOptions::default(),
    )?;
    let wip_docs = collect_documents(
        repo_root.join("PROJECT_TRACKING/WORK_IN_PROGRESS"),
        CollectOptions { skip_readme: true },
    )?;

    let completed_features: Vec<_> = feature_docs
        .iter()
        .filter(|doc| doc.is_completed())
        .cloned()
        .collect();
    let active_wips: Vec<_> = wip_docs
        .iter()
        .filter(|doc| doc.status.as_deref().map(is_active_status).unwrap_or(false))
        .cloned()
        .collect();
    let open_bugs: Vec<_> = bug_docs
        .iter()
        .filter(|doc| !doc.is_completed())
        .cloned()
        .collect();

    let build_outcome = if run_build {
        Some(run_command(
            "cargo",
            ["check", "--workspace", "--quiet"],
            "cargo check --workspace",
        ))
    } else {
        Some(CommandOutcome::skipped(
            "cargo check --workspace",
            "Skipped (set DEVIT_TRACKING_RUN_BUILD=1 to run)",
        ))
    };

    let test_outcome = if run_tests {
        Some(run_command(
            "cargo",
            ["test", "--workspace", "--no-run", "--quiet"],
            "cargo test --workspace --no-run",
        ))
    } else {
        Some(CommandOutcome::skipped(
            "cargo test --workspace --no-run",
            "Skipped (set DEVIT_TRACKING_RUN_TESTS=1 to run)",
        ))
    };

    let metrics = compute_metrics(&repo_root)?;
    let workspace_stats = match gather_workspace_stats() {
        Ok(stats) => stats,
        Err(err) => {
            eprintln!("⚠️  Unable to gather cargo metadata: {err}");
            WorkspaceStats::default()
        }
    };
    let git_branch = gather_git_output(["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|_| "unknown".to_string());
    let last_commit = gather_git_output(["log", "-1", "--pretty=%h\x09%s\x09%cr"])
        .unwrap_or_else(|_| "unavailable".to_string());

    let report = build_report(ReportContext {
        timestamp: timestamp.to_rfc3339(),
        feature_docs,
        completed_features,
        open_bugs,
        wip_docs,
        active_wips,
        build_outcome,
        test_outcome,
        metrics,
        workspace_stats,
        git_branch,
        last_commit,
        repo_root: repo_root.clone(),
    })?;

    if should_write {
        let output_path = repo_root.join("PROJECT_TRACKING/STATUS_REPORT.md");
        fs::write(&output_path, report.as_bytes())
            .with_context(|| format!("failed to write {}", output_path.display()))?;
        println!(
            "✅ Generated {} ({} bytes)",
            output_path.display(),
            report.len()
        );
    } else {
        println!("{report}");
    }

    Ok(())
}

#[derive(Clone, Default)]
struct CollectOptions {
    skip_readme: bool,
}

#[derive(Clone)]
struct DocumentStatus {
    title: String,
    status: Option<String>,
    completed: Option<String>,
    path: PathBuf,
}

impl DocumentStatus {
    fn is_completed(&self) -> bool {
        self.status
            .as_deref()
            .map(is_completed_status)
            .unwrap_or(false)
    }
}

#[derive(Clone)]
struct ReportContext {
    timestamp: String,
    feature_docs: Vec<DocumentStatus>,
    completed_features: Vec<DocumentStatus>,
    open_bugs: Vec<DocumentStatus>,
    wip_docs: Vec<DocumentStatus>,
    active_wips: Vec<DocumentStatus>,
    build_outcome: Option<CommandOutcome>,
    test_outcome: Option<CommandOutcome>,
    metrics: Metrics,
    workspace_stats: WorkspaceStats,
    git_branch: String,
    last_commit: String,
    repo_root: PathBuf,
}

#[derive(Clone)]
struct Metrics {
    total_files: usize,
    rust_files: usize,
    rust_loc: usize,
}

#[derive(Clone, Default)]
struct WorkspaceStats {
    workspace_members: usize,
    packages: usize,
}

#[derive(Clone)]
struct CommandOutcome {
    label: String,
    state: CommandState,
    duration_secs: Option<f64>,
    note: Option<String>,
}

#[derive(Clone)]
enum CommandState {
    Passed,
    Failed,
    Skipped,
}

impl CommandOutcome {
    fn success(label: impl Into<String>, duration_secs: f64) -> Self {
        Self {
            label: label.into(),
            state: CommandState::Passed,
            duration_secs: Some(duration_secs),
            note: None,
        }
    }

    fn failure(label: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            state: CommandState::Failed,
            duration_secs: None,
            note: Some(note.into()),
        }
    }

    fn skipped(label: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            state: CommandState::Skipped,
            duration_secs: None,
            note: Some(note.into()),
        }
    }

    fn summary(&self) -> String {
        match self.state {
            CommandState::Passed => {
                let duration = self
                    .duration_secs
                    .map(|secs| format!("{secs:.1}s"))
                    .unwrap_or_else(|| "?s".to_string());
                format!("✅ {} ({}).", self.label, duration)
            }
            CommandState::Failed => {
                let note = self
                    .note
                    .as_deref()
                    .unwrap_or("Command failed without additional diagnostics");
                format!("❌ {} — {}.", self.label, note)
            }
            CommandState::Skipped => {
                let note = self
                    .note
                    .as_deref()
                    .unwrap_or("Command skipped by configuration");
                format!("⚠️ {} — {}.", self.label, note)
            }
        }
    }
}

fn collect_documents(dir: PathBuf, options: CollectOptions) -> Result<Vec<DocumentStatus>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(OsStr::to_str) != Some("md") {
            continue;
        }
        if options.skip_readme
            && path
                .file_name()
                .and_then(OsStr::to_str)
                .map(|name| name.eq_ignore_ascii_case("README.md"))
                .unwrap_or(false)
        {
            continue;
        }
        let doc = parse_tracking_doc(&path)?;
        results.push(doc);
    }

    results.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    Ok(results)
}

fn parse_tracking_doc(path: &Path) -> Result<DocumentStatus> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut title: Option<String> = None;
    let mut status: Option<String> = None;
    let mut completed: Option<String> = None;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if title.is_none() && trimmed.starts_with('#') {
            let clean = trimmed.trim_start_matches('#').trim();
            if !clean.is_empty() {
                title = Some(clean.to_string());
            }
        }
        if status.is_none() {
            if let Some(value) = extract_field(trimmed, "Status") {
                status = Some(value);
            }
        }
        if completed.is_none() {
            if let Some(value) = extract_field(trimmed, "Completed") {
                completed = Some(value);
            }
        }
        if title.is_some() && status.is_some() && completed.is_some() {
            break;
        }
    }

    let final_title = title
        .or_else(|| {
            path.file_stem()
                .and_then(OsStr::to_str)
                .map(|s| s.replace('_', " "))
        })
        .unwrap_or_else(|| path.display().to_string());

    Ok(DocumentStatus {
        title: final_title,
        status,
        completed,
        path: path.to_path_buf(),
    })
}

fn extract_field(line: &str, field: &str) -> Option<String> {
    let marker = format!("**{field}:**");
    if line.starts_with(&marker) {
        let value = line[marker.len()..].trim();
        if value.is_empty() {
            None
        } else {
            Some(
                value
                    .trim_matches(|c| c == '*' || c == '`')
                    .trim()
                    .to_string(),
            )
        }
    } else {
        None
    }
}

fn build_report(ctx: ReportContext) -> Result<String> {
    use std::fmt::Write;

    let mut out = String::new();
    writeln!(out, "# 📊 DEVIT - STATUS REPORT")?;
    writeln!(out, "**Generated:** {}  ", ctx.timestamp)?;
    writeln!(out, "**Generator:** `devit-tools tracking_report`  ")?;
    writeln!(out, "**Git branch:** {}  ", ctx.git_branch)?;
    writeln!(out, "**Last commit:** {}  ", ctx.last_commit)?;
    writeln!(out, "\n---\n")?;

    writeln!(out, "## 🎯 Executive Summary\n")?;
    if let Some(build) = ctx.build_outcome {
        writeln!(out, "- {}", build.summary())?;
    }
    if let Some(test) = ctx.test_outcome {
        writeln!(out, "- {}", test.summary())?;
    }
    writeln!(
        out,
        "- Completed features: {} / {}.",
        ctx.completed_features.len(),
        ctx.feature_docs.len()
    )?;
    writeln!(
        out,
        "- Active WIP files: {} ({} total in directory).",
        ctx.active_wips.len(),
        ctx.wip_docs.len()
    )?;
    writeln!(out, "- Open bugs: {}.", ctx.open_bugs.len())?;
    writeln!(
        out,
        "- Workspace crates: {} ({} packages).",
        ctx.workspace_stats.workspace_members, ctx.workspace_stats.packages
    )?;
    writeln!(
        out,
        "- Rust LOC (approx): {} across {} files.",
        ctx.metrics.rust_loc, ctx.metrics.rust_files
    )?;
    writeln!(out, "\n---\n")?;

    writeln!(out, "## ✅ Completed Features\n")?;
    if ctx.completed_features.is_empty() {
        writeln!(out, "- *(none)*")?;
    } else {
        for feature in ctx.completed_features {
            let DocumentStatus {
                title,
                status,
                completed,
                path,
            } = feature;
            let mut line = format!("- {title}");
            if let Some(status) = status {
                line.push_str(&format!(" — {}", status));
            }
            if let Some(completed) = completed {
                line.push_str(&format!(" (Completed {completed})"));
            }
            line.push_str(&format!(" (`{}`)", display_path(&path, &ctx.repo_root)));
            writeln!(out, "{line}")?;
        }
    }
    writeln!(out, "\n---\n")?;

    writeln!(out, "## 🔄 Work In Progress\n")?;
    if ctx.wip_docs.is_empty() {
        writeln!(out, "- *(none)*")?;
    } else {
        for doc in ctx.wip_docs {
            let DocumentStatus {
                title,
                status,
                completed: _,
                path,
            } = doc;
            let status_text = status.unwrap_or_else(|| "Status unavailable".to_string());
            writeln!(
                out,
                "- {} — {} (`{}`)",
                title,
                status_text,
                display_path(&path, &ctx.repo_root)
            )?;
        }
    }
    writeln!(out, "\n---\n")?;

    writeln!(out, "## 🐛 Bugs & Issues\n")?;
    if ctx.open_bugs.is_empty() {
        writeln!(out, "- *(none)*")?;
    } else {
        for bug in ctx.open_bugs {
            let DocumentStatus {
                title,
                status,
                completed: _,
                path,
            } = bug;
            let status_text = status.unwrap_or_else(|| "Status unavailable".to_string());
            writeln!(
                out,
                "- {} — {} (`{}`)",
                title,
                status_text,
                display_path(&path, &ctx.repo_root)
            )?;
        }
    }
    writeln!(out, "\n---\n")?;

    writeln!(out, "## 📈 Metrics\n")?;
    writeln!(out, "- Total files scanned: {}.", ctx.metrics.total_files)?;
    writeln!(out, "- Rust source files: {}.", ctx.metrics.rust_files)?;
    writeln!(out, "- Approximate Rust LOC: {}.", ctx.metrics.rust_loc)?;
    writeln!(
        out,
        "- Workspace members: {} ({} packages in metadata).",
        ctx.workspace_stats.workspace_members, ctx.workspace_stats.packages
    )?;

    writeln!(
        out,
        "\n> Report generated automatically. Set `DEVIT_TRACKING_WRITE=0` to dry-run or `DEVIT_TRACKING_RUN_TESTS=1` to include test compilation results."
    )?;

    Ok(out)
}

fn display_path(path: &Path, repo_root: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn is_completed_status(status: &str) -> bool {
    let lower = status.to_ascii_lowercase();
    lower.contains("✅") || lower.contains("completed") || lower.contains("done")
}

fn is_active_status(status: &str) -> bool {
    let lower = status.to_ascii_lowercase();
    lower.contains("in_progress") || lower.contains("in progress") || lower.contains("active")
}

fn run_command<'a>(
    program: &str,
    args: impl IntoIterator<Item = &'a str>,
    label: &str,
) -> CommandOutcome {
    let start = Instant::now();
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(output) => {
            if output.status.success() {
                let duration = start.elapsed().as_secs_f64();
                CommandOutcome::success(label.to_string(), duration)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let snippet = stderr
                    .lines()
                    .next()
                    .unwrap_or("command failed without stderr output");
                CommandOutcome::failure(label.to_string(), snippet.trim().to_string())
            }
        }
        Err(err) => CommandOutcome::failure(label.to_string(), err.to_string()),
    }
}

fn compute_metrics(root: &Path) -> Result<Metrics> {
    let mut total_files = 0usize;
    let mut rust_files = 0usize;
    let mut rust_loc = 0usize;

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| !should_skip(entry))
    {
        let entry = entry?;
        if entry.file_type().is_file() {
            total_files += 1;
            if entry
                .path()
                .extension()
                .and_then(OsStr::to_str)
                .map(|ext| ext.eq_ignore_ascii_case("rs"))
                .unwrap_or(false)
            {
                rust_files += 1;
                rust_loc += count_non_empty_lines(entry.path())?;
            }
        }
    }

    Ok(Metrics {
        total_files,
        rust_files,
        rust_loc,
    })
}

fn should_skip(entry: &DirEntry) -> bool {
    let path = entry.path();
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            if matches!(
                name.to_str(),
                Some(
                    "target"
                        | ".git"
                        | ".idea"
                        | ".vscode"
                        | "node_modules"
                        | "vendor"
                        | ".venv"
                        | "__pycache__"
                )
            ) {
                return true;
            }
        }
    }
    false
}

fn count_non_empty_lines(path: &Path) -> Result<usize> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut count = 0usize;
    for line in reader.lines() {
        let line = line?;
        if !line.trim().is_empty() {
            count += 1;
        }
    }
    Ok(count)
}

fn gather_workspace_stats() -> Result<WorkspaceStats> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("running cargo metadata")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "cargo metadata failed: {}",
            stderr.lines().next().unwrap_or("unknown error")
        ));
    }

    let parsed: Value = serde_json::from_slice(&output.stdout).context("parsing cargo metadata")?;
    let packages = parsed
        .get("packages")
        .and_then(Value::as_array)
        .map(|arr| arr.len())
        .unwrap_or(0);
    let workspace_members = parsed
        .get("workspace_members")
        .and_then(Value::as_array)
        .map(|arr| arr.len())
        .unwrap_or(packages);

    Ok(WorkspaceStats {
        workspace_members,
        packages,
    })
}

fn gather_git_output(args: impl IntoIterator<Item = &'static str>) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("running git command")?;

    if !output.status.success() {
        return Err(anyhow!("git returned non-zero status"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}
