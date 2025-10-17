.PHONY: build test clean check lint example install

# Build targets
build:
	cargo build --release

debug:
	cargo build

# Test targets  
test:
	cargo test

test-verbose:
	cargo test -- --nocapture

# Quality checks
check:
	cargo check
	cargo clippy -- -D warnings
	cargo fmt --check

lint:
	cargo clippy -- -D warnings

fmt:
	cargo fmt

# Example usage
example: build
	@echo "=== Testing bad example ==="
	./target/release/analyze examples/bad_example.c -v
	@echo ""
	@echo "=== Testing good example ==="
	./target/release/analyze examples/good_example.c -v
	@echo ""
	@echo "=== Testing request command ==="
	echo "Analyze this C code for security issues" | ./target/release/request --file examples/bad_example.c

example-json: build
	./target/release/analyze examples/ --format json

# Installation
install: build
	cargo install --path .

# Cleanup
clean:
	cargo clean
	rm -f target/

# CI-like checks
ci: check test
	@echo "All checks passed!"

# Development workflow
dev: fmt check test example
	@echo "Development cycle complete!"Blaaa... ERROR !
