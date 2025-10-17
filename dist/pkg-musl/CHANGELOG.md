# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2025-01-21

### Added

- **Core DevIt CLI** (`devit`) with snapshot, apply, and policy management
- **MCP Server** (`devit-mcpd`) for Model Context Protocol integration
- **Enterprise Security Framework**:
  - **C4 Path Security** with canonicalization and symlink protection
  - **C2 Runtime Capabilities Detection** (bwrap, git, system limits)
  - Policy-based approval levels (untrusted, moderate, trusted)
  - Sandbox isolation with multiple security profiles
- **Advanced Error Handling**:
  - **C8 Detailed Error Taxonomy** with 7 categories (Validation, Security, State, Version, Operation, Resource, System)
  - Comprehensive recovery hints for all error types with actionable guidance
  - Context-aware error messages with line numbers and file paths
  - Enhanced debugging with severity levels and recoverability analysis
- **Schema Validation**:
  - **C10 Full MCP Request/Response Schema Validation**
  - Runtime JSON schema validation for all API endpoints
  - Custom validation rules with security integration
  - Tool-specific schemas for `devit.tool_call`, `plugin.invoke`, and server tools
- **Snapshot Management**:
  - **C6 High-performance snapshot creation** with BLAKE3 IDs
  - LRU caching for performance optimization
  - Pre-commit validation and TOCTOU protection
- **Patch Operations**:
  - Git-based patch parsing and application
  - Dry-run support with detailed preview
  - Security validation for all patch operations
- **Testing Framework**:
  - Multi-framework test detection (Cargo, npm, custom)
  - Sandbox integration for safe test execution
  - Intelligent test selection based on changed files
- **Configuration System**:
  - Flexible TOML-based configuration
  - Environment variable support
  - Per-project and global settings
- **CLI User Experience** (M5):
  - JSON output by default with `--pretty` option for human-readable format
  - `--dry-run` option for Apply command with explicit zero side-effect indication
  - Detailed summaries showing file counts, approval levels, and sandbox profiles
  - No stacktrace leaks in user mode with proper error handling
- **Versioning & Release** (M3):
  - Consistent versioning across binaries
  - Git hash inclusion in version output for both `devit` and `devit-mcpd`
  - Workspace-level version management
- **MCP Protocol Integration** (M1):
  - `devit-mcpd` binary for MCP protocol support
  - Core method implementations: `snapshot_get`, `patch_apply`, `test_run`, `journal_append`, `capabilities_get`
  - Input/output validation against JSON schemas
  - ISO 8601 timestamps and request ID echoing
  - Strict DevItError to error code mapping

### Changed
- **Project Status**: Upgraded from Alpha to Beta (production-ready)
- **Authors**: Updated to reflect three-way collaboration (N-Engine, GPT-5 Thinking, Claude)
- CLI argument structure: replaced `--json-only` with `--pretty` option (JSON is now default)
- Version output format now includes git hash for better traceability
- All workspace crates now use unified version management
- **Architecture**: Modular crate-based structure for scalability

### Fixed
- Path traversal vulnerabilities through strict validation
- Symlink escape attacks via boundary enforcement
- Race conditions in file operations (TOCTOU protection)
- Memory safety in snapshot operations
- Error handling edge cases across all modules
- Compilation errors in CLI output formatting and error handling
- Type mismatches in error details processing

### Security
- **Defense-in-Depth**: Multiple security layers for enterprise deployment
- **Audit Trail**: Cryptographic signatures for all operations
- **Boundary Enforcement**: Strict repository boundary validation
- **Input Validation**: Comprehensive sanitization of all inputs
- **Sandbox Isolation**: Process-level isolation for untrusted operations

## [Previous Development] - Sprint History

### Sprint 4 (In Progress)
- Overrides `--backend-url`, `--model`
- TUI: preview, approval interactive, watch; diff colorized; navigation
- CI/Release workflows

### Sprint 3
- `update_plan.yaml` (done/failed + JUnit summary + tail)
- Structured codeexec API (ExecResult, async, timeouts)

### Sprint 2
- Sandbox modes (read-only/workspace-write/danger) + safe-list
- Bwrap detection (`--unshare-net`), timeouts via env
- Route apply/run/test through Sandbox. JSONL logs

### Sprint 1
- Approval policy enum; config validation; decision table
- CLI enforcements for `apply/run` with `--yes` semantics

[Unreleased]: https://github.com/your-repo/devIt/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/your-repo/devIt/releases/tag/v0.1.0

