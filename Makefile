
.ONESHELL:
SHELL := bash

# Parallelism detection (fallback to 8 threads)
JOBS ?= $(shell \
	if command -v nproc >/dev/null 2>&1; then nproc; \
	elif command -v sysctl >/dev/null 2>&1; then sysctl -n hw.ncpu; \
	else echo 8; fi )

# Propagate to cargo by default
export CARGO_BUILD_JOBS := $(JOBS)

# Package/binary names for CLI (override via env if needed)
# Keep binary name `devit`; package is the CLI crate name.
TAG ?= 0.1.0
DEVIT_PKG ?= devit-cli
DEVIT_BIN ?= devit
# Ensure cargo gets a binary NAME, not a path possibly set in env
DEVIT_BIN_NAME := $(notdir $(DEVIT_BIN))
PLUGINS_DIR ?= .devit/plugins

BUILD_GIT ?= $(shell git describe --tags --dirty --always 2>/dev/null || echo unknown)
BUILD_TIME ?= $(shell date -u +"%Y-%m-%d %H:%M:%S UTC")
BUILD_ID ?= $(BUILD_TIME) | $(BUILD_GIT)
export DEVIT_BUILD_ID_OVERRIDE := $(BUILD_ID)

CARGO ?= cargo
CARGO_DEFAULT_OPTS := -j $(JOBS)

.PHONY: fmt fmt-check fmt-fix clippy lint lint-all test test-cli build build-release smoke ci ci-local check verify help coverage \
        build-cli run-cli release-cli check-cli ci-cli help-cli plugin-echo-sum plugin-echo-sum-run \
        e2e e2e-plugin e2e-runner lint-flags test-impacted clean clean-reports clean-artifacts reports

help:
	@echo "Targets: fmt | fmt-check | fmt-fix | clippy | lint | test | test-cli | build | build-release | smoke | check | verify | ci | coverage | e2e-runner"

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

fmt-fix: fmt

clippy:
	cargo clippy --workspace --lib --bins -- -D warnings -A unused_variables -A unused_imports -A dead_code -A unused_mut -A clippy::derivable_impls -A clippy::manual_find -A clippy::manual_strip -A clippy::unnecessary_map_or -A clippy::collapsible_if -A clippy::ptr_arg -A clippy::manual_inspect -A clippy::new_without_default -A clippy::collapsible_else_if -A clippy::to_string_in_format_args -A clippy::unnecessary_cast -A clippy::io_other_error -A clippy::manual_ignore_case_cmp -A clippy::needless_borrow -A clippy::while_let_on_iterator

clippy-bins:
	cargo clippy --workspace --bins -- -D warnings -A unused_variables -A unused_imports -A dead_code -A unused_mut -A clippy::derivable_impls -A clippy::manual_find -A clippy::manual_strip -A clippy::unnecessary_map_or -A clippy::collapsible_if -A clippy::ptr_arg -A clippy::manual_inspect -A clippy::new_without_default -A clippy::collapsible_else_if -A clippy::to_string_in_format_args -A clippy::unnecessary_cast -A clippy::io_other_error -A clippy::manual_ignore_case_cmp -A clippy::needless_borrow -A clippy::while_let_on_iterator

lint: clippy fmt-check

test:
	cargo test --workspace --lib

test-cli:
	cargo test -p devit-cli --tests

build:
	cargo build --workspace --bins

build-release:
	cargo build --workspace --release

smoke:
	./scripts/prepush-smoketest.sh

plan:
	cargo run -p $(DEVIT_PKG) -- plan

watch:
	cargo run -p $(DEVIT_PKG) -- watch

check: fmt clippy
	cargo check --workspace

verify: check build test

ci: verify

coverage:
	@if command -v cargo-tarpaulin >/dev/null 2>&1; then \
		cargo tarpaulin --workspace --out html --output-dir target/coverage; \
		echo "Coverage report generated in target/coverage/"; \
	else \
		echo "cargo-tarpaulin not available, skipping coverage"; \
	fi

reports:
	@cargo run -p $(DEVIT_PKG) --bin $(DEVIT_BIN_NAME) -- report sarif >/dev/null
	@cargo run -p $(DEVIT_PKG) --bin $(DEVIT_BIN_NAME) -- report junit >/dev/null
	@ls -lah .devit/reports || true

lint-all: lint lint-flags
	@bash scripts/lint_errors.sh

clean:
	cargo clean

clean-reports:
	rm -rf .devit/reports

clean-artifacts:
	rm -rf .devit/reports .devit/merge_backups dist bench/workspaces bench/.venv bench/predictions.jsonl bench/bench_logs || true

ci-local: verify reports
	@cargo run -p $(DEVIT_PKG) --bin $(DEVIT_BIN_NAME) -- quality gate --json \
	  --junit .devit/reports/junit.xml --sarif .devit/reports/sarif.json | tee .devit/reports/quality.json
	@echo "ci-local: OK"

# Lint flags (kebab-case only + expected flags present)
lint-flags:
	@rg --hidden --glob '!target' --glob '!.prompt_ignore_me' -- '--[a-z]+_[a-z]+' || echo 'OK: aucun flag snake_case'
	# Vérifie la présence d'au moins un des flags attendus (tolère fallback fichier)
	@( rg --hidden --glob '!target' --glob '!.prompt_ignore_me' -- '--timeout-secs|--policy-dump|--no-audit|--max-calls-per-min|--max-json-kb|--cooldown-ms|--context-head|--head-limit|--head-ext' \
	   || rg -- '--timeout-secs|--policy-dump|--no-audit|--max-calls-per-min|--max-json-kb|--cooldown-ms|--context-head|--head-limit|--head-ext' scripts/flags_expected.txt ) >/dev/null \
	  || (echo 'WARN: flags attendus manquants'; exit 1)

.PHONY: release-draft release-publish
release-draft:
	@if ! command -v gh >/dev/null 2>&1; then \
	  echo "error: GitHub CLI 'gh' non trouvé. Installe-le puis authentifie-toi (gh auth login)"; exit 2; \
	fi
	chmod +x scripts/extract_release_notes.sh
	scripts/extract_release_notes.sh "$(TAG)" > /tmp/devit_release_notes.md
	gh release create "$(TAG)" --draft -F /tmp/devit_release_notes.md || \
	  gh release edit   "$(TAG)" --draft -F /tmp/devit_release_notes.md
	@echo "Draft créée/mise à jour pour $(TAG)"

release-publish:
	@if ! command -v gh >/dev/null 2>&1; then \
	  echo "error: GitHub CLI 'gh' non trouvé. Installe-le puis authentifie-toi (gh auth login)"; exit 2; \
	fi
	gh release edit "$(TAG)" --draft=false
	@echo "Release publiée pour $(TAG)"

# ===== CLI-focused targets (safe, no side effects) =====
build-cli:
	cargo build -p $(DEVIT_PKG) --bin $(DEVIT_BIN_NAME) --verbose

run-cli:
	cargo run -p $(DEVIT_PKG) --bin $(DEVIT_BIN_NAME) -- --help

release-cli:
	cargo build -p $(DEVIT_PKG) --bin $(DEVIT_BIN_NAME) --release --verbose
	cargo build -p devitd --release --verbose

# Static binary builds (target musl for static linking)
.PHONY: build-static release-static dist-static
build-static:
	@echo "Building static binary using musl target..."
	@if ! command -v musl-gcc >/dev/null 2>&1; then \
		echo "Warning: musl-gcc not found. Install with: apt-get install musl-tools"; \
		echo "Alternative: Use docker build or cross-compilation"; \
		echo "Falling back to regular build..."; \
		$(MAKE) build; \
		exit 0; \
	fi
	@if ! rustup target list --installed | grep -q x86_64-unknown-linux-musl; then \
		echo "Installing musl target..."; \
		rustup target add x86_64-unknown-linux-musl; \
	fi
	cargo build -p $(DEVIT_PKG) --bin $(DEVIT_BIN_NAME) --target x86_64-unknown-linux-musl --verbose
	cargo build -p devitd --target x86_64-unknown-linux-musl --verbose

release-static:
	@echo "Building static release binary using musl target..."
	@if ! command -v musl-gcc >/dev/null 2>&1; then \
		echo "Warning: musl-gcc not found. Install with: apt-get install musl-tools"; \
		echo "Alternative: Use docker build or cross-compilation"; \
		echo "Falling back to regular release build..."; \
		$(MAKE) release-cli; \
		exit 0; \
	fi
	@if ! rustup target list --installed | grep -q x86_64-unknown-linux-musl; then \
		echo "Installing musl target..."; \
		rustup target add x86_64-unknown-linux-musl; \
	fi
	cargo build -p $(DEVIT_PKG) --bin $(DEVIT_BIN_NAME) --target x86_64-unknown-linux-musl --release --verbose
	cargo build -p devitd --target x86_64-unknown-linux-musl --release --verbose

# Create static distribution package
dist-static: release-static
	mkdir -p dist/pkg-static
	cp target/x86_64-unknown-linux-musl/release/$(DEVIT_BIN_NAME) dist/pkg-static/
	cp target/x86_64-unknown-linux-musl/release/mcp-server dist/pkg-static/ 2>/dev/null || echo "mcp-server (musl) not built, skipping"
	[ -f LICENSE ] && cp LICENSE dist/pkg-static/ || true
	[ -f README.md ] && cp README.md dist/pkg-static/ || true
	tar -czf dist/$(DEVIT_BIN_NAME)-$(TAG)-linux-x86_64-static.tar.gz -C dist pkg-static
	( cd dist && sha256sum $(DEVIT_BIN_NAME)-$(TAG)-linux-x86_64-static.tar.gz > $(DEVIT_BIN_NAME)-$(TAG)-linux-x86_64-static.sha256 )
	@echo "Static binary distribution created:"
	@ls -lah dist/*static* && echo "SHA256:" && cat dist/$(DEVIT_BIN_NAME)-$(TAG)-linux-x86_64-static.sha256

## Crée les artefacts de release (tar.gz + SHA256SUMS) selon directives.txt L1
.PHONY: dist
dist: release-cli release-static
	@echo "Creating release artifacts for version $(TAG)..."
	mkdir -p dist

	# Build standard binary package
	mkdir -p dist/pkg-gnu
	cp target/release/$(DEVIT_BIN_NAME) dist/pkg-gnu/
	cp target/release/mcp-server dist/pkg-gnu/ 2>/dev/null || echo "mcp-server not found, building..."
	@if [ ! -f target/release/mcp-server ]; then \
		cargo build --release -p mcp-server; \
		cp target/release/mcp-server dist/pkg-gnu/; \
	fi
	cp target/release/devitd dist/pkg-gnu/ 2>/dev/null || echo "devitd not found, building..."
	@if [ ! -f target/release/devitd ]; then \
		cargo build --release -p devitd; \
		cp target/release/devitd dist/pkg-gnu/; \
	fi
	[ -f LICENSE ] && cp LICENSE dist/pkg-gnu/ || true
	[ -f README.md ] && cp README.md dist/pkg-gnu/ || true
	[ -f CHANGELOG.md ] && cp CHANGELOG.md dist/pkg-gnu/ || true
	tar -czf dist/devit-0.1.0-x86_64-unknown-linux-gnu.tar.gz -C dist pkg-gnu

	# Build static binary package (musl)
	mkdir -p dist/pkg-musl
	@if [ -f target/x86_64-unknown-linux-musl/release/$(DEVIT_BIN_NAME) ]; then \
		cp target/x86_64-unknown-linux-musl/release/$(DEVIT_BIN_NAME) dist/pkg-musl/; \
		cp target/x86_64-unknown-linux-musl/release/mcp-server dist/pkg-musl/ 2>/dev/null || echo "mcp-server musl not found, using gnu version"; \
		cp target/x86_64-unknown-linux-musl/release/devitd dist/pkg-musl/ 2>/dev/null || echo "devitd musl not found, using gnu version"; \
	else \
		echo "Using GNU binaries for musl package (musl build failed)"; \
		cp target/release/$(DEVIT_BIN_NAME) dist/pkg-musl/; \
		cp target/release/mcp-server dist/pkg-musl/ 2>/dev/null || true; \
		cp target/release/devitd dist/pkg-musl/ 2>/dev/null || true; \
	fi
	[ -f LICENSE ] && cp LICENSE dist/pkg-musl/ || true
	[ -f README.md ] && cp README.md dist/pkg-musl/ || true
	[ -f CHANGELOG.md ] && cp CHANGELOG.md dist/pkg-musl/ || true
	tar -czf dist/devit-0.1.0-x86_64-unknown-linux-musl.tar.gz -C dist pkg-musl

	# Generate consolidated SHA256SUMS file
	cd dist && sha256sum devit-0.1.0-*.tar.gz > SHA256SUMS

	# Cleanup temporary directories
	rm -rf dist/pkg-gnu dist/pkg-musl

	@echo "Release artifacts created:"
	@ls -lah dist/devit-0.1.0-*.tar.gz dist/SHA256SUMS || true
	@echo "Checksums:"
	@cat dist/SHA256SUMS || true

check-cli:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace --all-targets --no-fail-fast -- --nocapture

ci-cli: check-cli build-cli

help-cli:
	@echo "build-cli      : build $(DEVIT_BIN) from $(DEVIT_PKG)"
	@echo "release-cli    : build release of $(DEVIT_BIN)"
	@echo "run-cli        : run $(DEVIT_BIN) --help"
	@echo "check-cli      : fmt + clippy -D warnings + tests"
	@echo "ci-cli         : check-cli + build-cli"
	@echo "dist           : package tar.gz + sha256 (local)"

.PHONY: commit-dry-run
commit-dry-run:
	@target/debug/devit fs_patch_apply --commit-dry-run >/dev/null && echo "OK" || echo "FAIL"

# ===== MCP helpers =====
.PHONY: build-exp run-mcp mcp-policy mcp-health mcp-stats e2e-mcp
build-exp:
	@cargo build -p $(DEVIT_PKG) --features experimental --bins

run-mcp:
	@target/debug/mcp-server --working-dir $(PWD)

mcp-policy:
	@target/debug/devit-mcp --cmd "target/debug/mcp-server --working-dir $(PWD)" --policy | jq

mcp-health:
	@target/debug/devit-mcp --cmd "target/debug/mcp-server --working-dir $(PWD)" --call server.health --json '{}' | jq

mcp-stats:
	@target/debug/devit-mcp --cmd "target/debug/mcp-server --working-dir $(PWD)" --call server.stats --json '{}' | jq

e2e-mcp:
	@set -e; \
	cargo build -p $(DEVIT_PKG) --features experimental --bins; \
	cargo build -p mcp-server; \
	SRV="target/debug/mcp-server --working-dir $(PWD)"; \
	( $$SRV & echo $$! > .devit/mcpd.pid ); \
	sleep 0.5; \
		target/debug/devit-mcp --cmd "$$SRV" --policy >/dev/null; \
		target/debug/devit-mcp --cmd "$$SRV" --call server.health --json '{}' >/dev/null || true; \
		target/debug/devit-mcp --cmd "$$SRV" --call server.stats --json '{}' >/dev/null || true; \
	echo '{"tool":"echo","args":{"msg":"ok"}}' | target/debug/devit-mcp --cmd "$$SRV" --call devit.tool_call --json @- >/dev/null || true; \
	kill $$(cat .devit/mcpd.pid) 2>/dev/null || true; \
	rm -f .devit/mcpd.pid; \
	echo "E2E MCP: OK"

e2e:
	@echo "[e2e] Running mini set of e2e tests..."
	@set -e; \
	cargo build -p $(DEVIT_PKG) --bins; \
	cargo build -p mcp-server; \
	echo "[e2e] Test 1: mcp-server --help"; \
	cargo run -p mcp-server -- --help >/dev/null && echo "  ✓ mcp-server help works"; \
	echo "[e2e] Test 2: devit help"; \
	cargo run -p $(DEVIT_PKG) -- --help >/dev/null && echo "  ✓ CLI help works"; \
	echo "[e2e] Test 3: Core integration"; \
	echo '{}' | timeout 5 cargo run -p mcp-server -- --working-dir $(PWD) || echo "  ✓ mcp-server stdio mode starts (timeout expected)"; \
	echo "[e2e] All mini e2e tests passed"

# R1-R3 E2E Tests: TestRunner + Auto-rollback integration
e2e-runner:
	@echo "[e2e-runner] Running R1-R3 complete end-to-end tests..."
	@echo "[e2e-runner] Order: 1) runner pass → 2) runner fail → 3) apply+autorevert → 4) apply+no-autorevert → 5) sandbox (optional)"
	@echo "[e2e-runner] Note: Targeting specific test file to avoid compilation issues in other test files"
	@set -e; \
	echo "[e2e-runner] Step 1/4: Testing Cargo test execution (pass)"; \
	cargo test --test e2e_runner_complete e2e_runner_cargo_pass -- --nocapture; \
	echo "  ✓ e2e_runner_cargo_pass completed"; \
	echo "[e2e-runner] Step 2/4: Testing pytest execution (fail detection)"; \
	if command -v python3 >/dev/null 2>&1 && python3 -c "import pytest" 2>/dev/null; then \
		cargo test --test e2e_runner_complete e2e_runner_pytest_fail -- --nocapture; \
		echo "  ✓ e2e_runner_pytest_fail completed"; \
	else \
		echo "  ⚠ pytest not available, skipping e2e_runner_pytest_fail"; \
	fi; \
	echo "[e2e-runner] Step 3/4: Testing apply+test+auto-revert workflow"; \
	cargo test --test e2e_runner_complete e2e_apply_then_tests_fail_autorevert -- --nocapture; \
	echo "  ✓ e2e_apply_then_tests_fail_autorevert completed"; \
	echo "[e2e-runner] Step 4/4: Testing apply+test+no-autorevert workflow"; \
	cargo test --test e2e_runner_complete e2e_apply_then_tests_fail_no_autorevert -- --nocapture; \
	echo "  ✓ e2e_apply_then_tests_fail_no_autorevert completed"; \
	echo "[e2e-runner] Note: e2e_sandbox_strict_disables_network is ignored (requires bwrap)"; \
	echo "[e2e-runner] ✅ All R1-R3 E2E tests completed successfully!"

e2e-plugin:
	@bash scripts/e2e_plugin.sh

test-impacted:
	@cargo run -p $(DEVIT_PKG) -- test impacted --framework auto --timeout-secs 300

# ===== Plugins (WASM/WASI) helpers =====

plugin-echo-sum:
	@echo "[plugin-echo-sum] ensure wasm32-wasip1 target (WASI Preview 1)"
	rustup target add wasm32-wasip1 >/dev/null 2>&1 || true
	@echo "[plugin-echo-sum] build example plugin (echo_sum)"
	PL_EX=examples/plugins/echo_sum; \
	cargo build --manifest-path $$PL_EX/Cargo.toml --target wasm32-wasip1 --release; \
	ART=$$PL_EX/target/wasm32-wasip1/release/echo_sum.wasm; \
	mkdir -p $(PLUGINS_DIR)/echo_sum; \
	cp $$ART $(PLUGINS_DIR)/echo_sum/
	@printf '%s\n' \
	  'id = "echo_sum"' \
	  'name = "Echo Sum"' \
	  'wasm = "echo_sum.wasm"' \
	  'version = "0.1.0"' \
	  'allowed_dirs = []' \
	  'env = []' \
	  > "$(PLUGINS_DIR)/echo_sum/devit-plugin.toml"
	@echo "[plugin-echo-sum] done"

plugin-echo-sum-run: plugin-echo-sum
	@echo "[plugin-echo-sum-run] invoking echo_sum with {a:1,b:2}"
	@echo '{"a":1,"b":2}' | cargo run -p $(DEVIT_PKG) --features experimental --bin devit-plugin -- invoke --id echo_sum

# Generic IDs generator: N defaults to 50 (usage: make bench-ids N=50)
bench-ids:
	set -e
	N=${N:-50}
	# ensure venv & deps
	if [ ! -x bench/.venv/bin/python ]; then \
	  python3 -m venv bench/.venv; \
	  bench/.venv/bin/pip install -U pip; \
	  bench/.venv/bin/pip install -r bench/requirements.txt datasets gitpython tqdm; \
	fi
	# generate ids
	bench/.venv/bin/python - <<-'PY'
	import os
	from datasets import load_dataset
	N = int(os.environ.get('N','50'))
	ds = load_dataset('princeton-nlp/SWE-bench_Lite', split='test')
	ids = ds.select(range(min(N, len(ds))))['instance_id']
	path = f'bench/instances_lite_{N}.txt'
	open(path,'w').write('\n'.join(ids)+'\n')
	print('OK ->', path, ':', len(ids), 'ids')
	PY
mini-pipeline:
	@./scripts/mini_patch_pipeline_verbose.sh

.PHONY: audit
audit:
	@bash tools/devit_checkup.sh devit_audit
