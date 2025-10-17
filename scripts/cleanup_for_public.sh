#!/bin/bash
# ================================================================
# DEVIT REPO CLEANUP - Pre-Public Release Sanitization
# ================================================================
# Purpose: Clean the repo before public push without losing work
# Author: Orchestrator + Human
# Date: 2025-10-14
# ================================================================

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}     DevIt Repository Cleanup - Pre-Public Release${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo

# ================================================================
# STEP 0: SAFETY CHECKS
# ================================================================
echo -e "${YELLOW}[0/7]${NC} Safety checks..."

# Check if we're in devIt root
if [[ ! -f "Cargo.toml" ]] || [[ ! -d "crates" ]]; then
    echo -e "${RED}ERROR: Not in devIt root directory!${NC}"
    exit 1
fi

# Check for uncommitted changes
if [[ -n $(git status --porcelain) ]]; then
    echo -e "${RED}ERROR: Uncommitted changes detected!${NC}"
    echo "Please commit or stash changes first:"
    echo "  git add . && git commit -m 'WIP: pre-cleanup checkpoint'"
    echo "  OR"
    echo "  git stash push -m 'pre-cleanup stash'"
    exit 1
fi

# Create safety backup branch
BACKUP_BRANCH="backup/pre-public-cleanup-$(date +%Y%m%d-%H%M%S)"
echo -e "${GREEN}Creating backup branch: ${BACKUP_BRANCH}${NC}"
git branch "$BACKUP_BRANCH"

# ================================================================
# STEP 1: ARCHIVE IMPORTANT FILES
# ================================================================
echo -e "${YELLOW}[1/7]${NC} Archiving important files..."

# Create archive structure
mkdir -p .archive/{tests,audits,logs,screenshots,patches,misc}
mkdir -p .archive/legacy/{binaries,demos,benchmarks}

# Move test files (but keep essential ones)
echo "  Moving test artifacts..."
mv test_*.txt .archive/tests/ 2>/dev/null || true
mv test_*.rs .archive/tests/ 2>/dev/null || true  
mv test_*.json .archive/tests/ 2>/dev/null || true
mv test_*.log .archive/tests/ 2>/dev/null || true

# Move audit files
echo "  Moving audit files..."
mv audit*.txt .archive/audits/ 2>/dev/null || true
mv audit*.md .archive/audits/ 2>/dev/null || true
mv *_audit.txt .archive/audits/ 2>/dev/null || true
mv WINDOWS_AUDIT*.md .archive/audits/ 2>/dev/null || true

# Move logs
echo "  Moving logs..."
mv *.log .archive/logs/ 2>/dev/null || true

# Move screenshots  
echo "  Moving screenshots..."
mv *.png .archive/screenshots/ 2>/dev/null || true
mv *.jpg .archive/screenshots/ 2>/dev/null || true

# Move patches
echo "  Moving patches..."
mv *.patch .archive/patches/ 2>/dev/null || true

# Move archives
echo "  Moving compressed archives..."
mv *.tar.gz .archive/misc/ 2>/dev/null || true
mv *.zip .archive/misc/ 2>/dev/null || true

# Move perf/stress test files
echo "  Moving performance test files..."
mv PERF_*.txt .archive/misc/ 2>/dev/null || true
mv stress_*.txt .archive/misc/ 2>/dev/null || true
mv chaos_*.csv .archive/misc/ 2>/dev/null || true
mv baseline.csv .archive/misc/ 2>/dev/null || true

# Move random check files
echo "  Moving check files..."
mv check_*.txt .archive/misc/ 2>/dev/null || true
mv check_*.sh .archive/misc/ 2>/dev/null || true

# Move other misc files
echo "  Moving miscellaneous files..."
mv hello_*.txt .archive/misc/ 2>/dev/null || true
mv server.lo .archive/misc/ 2>/dev/null || true
mv malformed.json .archive/misc/ 2>/dev/null || true
mv medium_test_file.txt .archive/misc/ 2>/dev/null || true
mv sample_result.txt .archive/misc/ 2>/dev/null || true
mv tree_before_screenshots.txt .archive/misc/ 2>/dev/null || true
mv directives_*.txt .archive/misc/ 2>/dev/null || true
mv migration.txt .archive/misc/ 2>/dev/null || true
mv reprise.txt .archive/misc/ 2>/dev/null || true
mv deletion_list.txt .archive/misc/ 2>/dev/null || true
mv permissions-audit.txt .archive/misc/ 2>/dev/null || true
mv adv.txt .archive/misc/ 2>/dev/null || true
mv agent.md .archive/misc/ 2>/dev/null || true
mv roadmap_debug.txt .archive/misc/ 2>/dev/null || true
mv win_port*.txt .archive/misc/ 2>/dev/null || true

# Move legacy directories
echo "  Moving legacy directories..."
[[ -d "devit-approver" ]] && mv devit-approver .archive/legacy/binaries/ 2>/dev/null || true
[[ -d "devit-bench" ]] && mv devit-bench .archive/legacy/benchmarks/ 2>/dev/null || true
[[ -d "devit-chaos" ]] && mv devit-chaos .archive/legacy/benchmarks/ 2>/dev/null || true
[[ -d "devit-jverify" ]] && mv devit-jverify .archive/legacy/binaries/ 2>/dev/null || true
[[ -d "devit-patch-fix" ]] && mv devit-patch-fix .archive/legacy/binaries/ 2>/dev/null || true
[[ -d "devit_audit" ]] && mv devit_audit .archive/legacy/binaries/ 2>/dev/null || true
[[ -d "devitd-client" ]] && mv devitd-client .archive/legacy/binaries/ 2>/dev/null || true

# Remove Windows garbage
echo "  Removing Windows artifacts..."
rm -rf "System Volume Information" 2>/dev/null || true

# Move standalone scripts that shouldn't be at root
echo "  Moving standalone scripts..."
mv devit_mcp_standalone* .archive/misc/ 2>/dev/null || true
mv test_client.rs .archive/misc/ 2>/dev/null || true
mv test_devit_write.rs .archive/misc/ 2>/dev/null || true
mv test_llm_patch.rs .archive/misc/ 2>/dev/null || true
mv test_patch_real.rs .archive/misc/ 2>/dev/null || true
mv test_patch_simple* .archive/misc/ 2>/dev/null || true

# Move Python analysis scripts (keep if useful)
mkdir -p tools/analysis
mv analyze_bench_results.py tools/analysis/ 2>/dev/null || true
mv simple_bench_analysis.py tools/analysis/ 2>/dev/null || true
mv rust_analyzer.py tools/analysis/ 2>/dev/null || true

# Move benchmark/test runners (keep if useful)
mkdir -p scripts/testing
mv run_bench_suite.sh scripts/testing/ 2>/dev/null || true
mv test_approval_workflow.sh scripts/testing/ 2>/dev/null || true
mv test_chaos.sh scripts/testing/ 2>/dev/null || true
mv validate_*.sh scripts/testing/ 2>/dev/null || true

echo -e "${GREEN}✓ Files archived to .archive/${NC}"

# ================================================================
# STEP 2: CREATE PROPER .gitignore
# ================================================================
echo -e "${YELLOW}[2/7]${NC} Creating comprehensive .gitignore..."

cat > .gitignore << 'EOF'
# Rust build artifacts
/target/
**/*.rs.bk
Cargo.lock

# DevIt runtime
/.devit/
/devitd.journal
/.devit-snapshots/

# IDE
.idea/
.vscode/
*.swp
*.swo
*~
.DS_Store

# Logs and debug
*.log
*.trace
debug.txt

# Test artifacts
test_*.txt
test_*.rs
test_*.json
*.test
coverage/

# Temporary files
*.tmp
*.temp
*.bak
*.old
tmp/
temp/

# Archives
*.tar.gz
*.tar.bz2
*.zip
*.7z

# Performance/benchmark
perf_*.txt
stress_*.txt
bench_*.json
*.csv

# Screenshots and media
*.png
*.jpg
*.jpeg
*.gif
*.mp4
*.avi

# Python
__pycache__/
*.py[cod]
*$py.class
*.so
.Python
env/
venv/

# Archive directory (cleanup artifacts)
.archive/

# Windows
Thumbs.db
ehthumbs.db
Desktop.ini
$RECYCLE.BIN/
System Volume Information/

# macOS
.DS_Store
.AppleDouble
.LSOverride
._*

# Audit files (keep in PROJECT_TRACKING)
audit_*.txt
audit_*.md
*_audit.txt

# Local config overrides
*.local.toml
devit.user.toml

# Credentials (NEVER commit)
*.key
*.pem
*.cert
credentials.json
secrets.toml
EOF

echo -e "${GREEN}✓ .gitignore created${NC}"

# ================================================================
# STEP 3: CLEAN DOCUMENTATION
# ================================================================
echo -e "${YELLOW}[3/7]${NC} Organizing documentation..."

# Keep only essential docs at root
KEEP_ROOT_DOCS=(
    "README.md"
    "LICENSE"
    "CONTRIBUTING.md"
    "CODE_OF_CONDUCT.md"
    "SECURITY.md"
    "CHANGELOG.md"
)

# Move other docs to docs/
mkdir -p docs/archive
for file in *.md; do
    if [[ ! " ${KEEP_ROOT_DOCS[@]} " =~ " ${file} " ]]; then
        if [[ -f "$file" ]]; then
            echo "  Moving $file to docs/"
            mv "$file" docs/archive/ 2>/dev/null || true
        fi
    fi
done

# Move .old files
mv *.old .archive/misc/ 2>/dev/null || true

echo -e "${GREEN}✓ Documentation organized${NC}"

# ================================================================
# STEP 4: VALIDATE STRUCTURE
# ================================================================
echo -e "${YELLOW}[4/7]${NC} Validating repository structure..."

# Expected structure
EXPECTED_DIRS=("crates" "docs" "scripts" "examples" "tests" "PROJECT_TRACKING")
EXPECTED_FILES=("Cargo.toml" "README.md" "LICENSE" ".gitignore")

echo "  Checking essential directories..."
for dir in "${EXPECTED_DIRS[@]}"; do
    if [[ -d "$dir" ]]; then
        echo -e "    ${GREEN}✓${NC} $dir"
    else
        echo -e "    ${RED}✗${NC} $dir (missing)"
    fi
done

echo "  Checking essential files..."
for file in "${EXPECTED_FILES[@]}"; do
    if [[ -f "$file" ]]; then
        echo -e "    ${GREEN}✓${NC} $file"
    else
        echo -e "    ${RED}✗${NC} $file (missing)"
    fi
done

# ================================================================
# STEP 5: FIX TODOs IN CRITICAL FILES
# ================================================================
echo -e "${YELLOW}[5/7]${NC} Scanning for remaining TODOs..."

echo "  Critical TODOs found:"
grep -rn "todo!()" crates/ 2>/dev/null | grep -v test | head -5 || true
grep -rn "unimplemented!()" crates/ 2>/dev/null | grep -v test | head -5 || true

echo -e "${YELLOW}  Note: These need manual fixing before public release${NC}"

# ================================================================  
# STEP 6: CREATE CLEAN COMMIT
# ================================================================
echo -e "${YELLOW}[6/7]${NC} Creating cleanup commit..."

# Add all changes
git add -A

# Create detailed commit message
git commit -m "chore: repository cleanup for public release

- Archived test artifacts and temporary files to .archive/
- Reorganized documentation structure
- Created comprehensive .gitignore
- Moved analysis tools to tools/analysis/
- Moved test scripts to scripts/testing/
- Removed Windows system files
- Cleaned root directory (121 -> ~15 files)

Previous state backed up to: $BACKUP_BRANCH"

echo -e "${GREEN}✓ Cleanup committed${NC}"

# ================================================================
# STEP 7: FINAL REPORT
# ================================================================
echo -e "${YELLOW}[7/7]${NC} Generating cleanup report..."

echo
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}                   CLEANUP COMPLETE${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo

echo "📊 Statistics:"
echo "  Root files before: ~121"
echo "  Root files after:  $(ls -1 | wc -l)"
echo "  Files archived:    $(find .archive -type f 2>/dev/null | wc -l)"
echo "  Backup branch:     $BACKUP_BRANCH"

echo
echo "📝 Next steps before public push:"
echo "  1. Fix remaining TODOs in critical files"
echo "  2. Review git history (consider squashing commits):"
echo "     git rebase -i HEAD~50"
echo "  3. Test build:"
echo "     cargo build --release"
echo "     cargo test --workspace"
echo "  4. Update README.md with:"
echo "     - Clear project description"
echo "     - Installation instructions"
echo "     - Current status (alpha/beta)"
echo "  5. Create initial release tag:"
echo "     git tag -a v0.1.0-alpha -m 'Initial public release'"

echo
echo -e "${YELLOW}⚠️  IMPORTANT:${NC}"
echo "  - Backup branch created: $BACKUP_BRANCH"
echo "  - Archived files in: .archive/"
echo "  - To undo: git reset --hard $BACKUP_BRANCH"

echo
echo -e "${GREEN}✨ Repository is ready for review before public push!${NC}"
