#!/usr/bin/env bash
set -euo pipefail

OUT="${1:-devit_audit}"
mkdir -p "$OUT"

echo "[1/12] Repo map & sizes"
# tokei pour SLOC
tokei -o json . > "$OUT/sloc.json" || true

echo "[2/12] Project structure (if tool available)"
devit project-structure --json > "$OUT/project_structure.json" 2>/dev/null || true

echo "[3/12] File list (compressed ext/table if available)"
devit file-list --json > "$OUT/file_list.json" 2>/dev/null || true

echo "[4/12] TODO/FIXME/UNIMPLEMENTED"
rg -n --json -e 'TODO|FIXME|UNIMPLEMENTED|panic!\(|unimplemented!\(|#[ignore]' \
  > "$OUT/todos.json" || true

echo "[5/12] Ignored tests"
rg -n --json '#\[ignore' crates | jq -s '.' > "$OUT/ignored_tests.json" || true

echo "[6/12] Clippy"
cargo clippy --all-targets --message-format=json \
  > "$OUT/clippy.json" || true

echo "[7/12] Fmt"
cargo fmt --all -- --check >/dev/null 2>&1 || echo "fmt: not formatted" > "$OUT/fmt.txt"

echo "[8/12] Unused dependencies"
cargo +stable udeps --all-targets --output json \
  > "$OUT/udeps.json" || true

echo "[9/12] Licences & vulns"
cargo deny check -q || true
cargo deny check -L error -o json > "$OUT/deny.json" || true
cargo audit -q -F json > "$OUT/audit.json" || true

echo "[10/12] Tests + coverage (if tarpaulin/llvm-cov)"
cargo nextest run --serialize-junit > "$OUT/nextest.junit.xml" || true
# llvm-cov (if installed)
cargo llvm-cov --json --output-path "$OUT/coverage.json" || true

echo "[11/12] Binaries & tailles"
cargo build --release
ls -lh target/release | awk '{print $5,$9}' > "$OUT/bin_sizes.txt"

echo "[12/12] MCP tools exposed"
devit mcp list --json > "$OUT/mcp_tools.json" 2>/dev/null || true

echo "[CHUNK] Automatic chunk split"
python3 - "$OUT" <<'PY'
import sys, json, os, pathlib
out = pathlib.Path(sys.argv[1])
for p in out.iterdir():
    if p.stat().st_size > 180_000 and p.suffix in {".json",".txt",".xml",".csv"}:
        data = p.read_text(encoding="utf-8", errors="ignore")
        for i in range(0, len(data), 180_000):
            (out / f"{p.stem}.part{int(i/180000):03d}{p.suffix}").write_text(
                data[i:i+180_000], encoding="utf-8")
PY

echo "✅ Audit ready in: $OUT"
