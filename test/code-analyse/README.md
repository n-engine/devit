# Code Analyzer

Rust reimplementation of C code analysis bash scripts. This project serves as a testing ground for the **DevIt** AI collaboration tool.

## Overview

This tool provides two main commands that replicate the functionality of bash scripts:

- `request` - LLM request wrapper (equivalent to `request` bash script)
- `analyze` - C code analyzer (equivalent to `analyse_code.sh` bash script)

## Installation

```bash
# Build the project
make build

# Or install globally
make install
```

## Usage

### Code Analysis

```bash
# Analyze a single file
./target/release/analyze examples/bad_example.c

# Analyze a directory
./target/release/analyze examples/ --verbose

# Get JSON output
./target/release/analyze examples/ --format json

# Use custom rules
./target/release/analyze examples/ --rules custom_rules.txt

# Strict mode (exit with error if issues found)
./target/release/analyze examples/ --strict
```

### LLM Requests

```bash
# Direct prompt
./target/release/request "Explain this code" --file examples/bad_example.c

# From stdin
echo "What are the security issues in this code?" | ./target/release/request --file examples/bad_example.c

# Different model
./target/release/request "Analyze this code" --model llama3.1 --file examples/bad_example.c

# Raw JSON output
./target/release/request "Test prompt" --raw
```

## Rules Configuration

The analyzer uses a simple rule format in `rules.txt`:

```
RULE_ID|SEVERITY|PATTERN|DESCRIPTION|SUGGESTION
```

Example:
```
no_gets|error|\bgets\s*\(|Never use gets() function|Use fgets() instead
```

Supported severities: `error`, `warning`, `info`

## Testing with DevIt

This project is designed to test DevIt's capabilities:

1. **Code improvements**: DevIt can suggest patches to fix detected issues
2. **Rule additions**: Add new analysis rules through AI collaboration  
3. **Refactoring**: Improve code structure with AI-assisted changes
4. **Testing**: Automated test generation and validation

## Examples

The `examples/` directory contains:

- `bad_example.c` - Demonstrates various code issues
- `good_example.c` - Shows proper C coding practices

Run `make example` to see the analyzer in action.

## Development

```bash
# Run all checks
make ci

# Development cycle
make dev

# Format code
make fmt

# Run tests
make test
```

## Architecture

- `src/main.rs` - CLI entry point and command routing
- `src/analyzer.rs` - Core analysis engine
- `src/rules.rs` - Rule parsing and execution engine  
- `src/llm.rs` - LLM client (with mock implementation)
- `src/bin/` - Standalone binary commands

## License

Apache-2.0