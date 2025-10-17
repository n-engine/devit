#!/bin/bash
# ================================================================
# GIT HISTORY ANALYZER - Find Problematic Commits
# ================================================================
# Purpose: Identify commits that should be removed/squashed
# ================================================================

set -euo pipefail

# Colors
RED='\033[0;31m'
YELLOW='\033[1;33m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
NC='\033[0m'

echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}         Git History Analysis - Pre-Public Audit${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo

# ================================================================
# FIND LARGE COMMITS (potential monoliths)
# ================================================================
echo -e "${YELLOW}[1/6]${NC} Searching for large commits (>10K changes)..."
echo

git log --oneline --numstat | awk '
    /^[0-9a-f]{7}/ {
        if (additions > 10000 || deletions > 10000) {
            printf "  ⚠️  %s: +%d -%d %s\n", commit, additions, deletions, message
        }
        commit = $1
        message = substr($0, 9)
        additions = 0
        deletions = 0
    }
    /^[0-9]+\t[0-9]+/ {
        additions += $1
        deletions += $2
    }
    END {
        if (additions > 10000 || deletions > 10000) {
            printf "  ⚠️  %s: +%d -%d %s\n", commit, additions, deletions, message
        }
    }
' | head -20

# ================================================================
# FIND SUSPICIOUS COMMIT MESSAGES
# ================================================================
echo
echo -e "${YELLOW}[2/6]${NC} Searching for suspicious commit messages..."
echo

echo "  WIP/Test commits:"
git log --oneline | grep -iE "WIP|test|tmp|temp|TODO|FIXME|hack|quick|dirty|broken|fuck|shit|crap" | head -10 || echo "    None found"

echo
echo "  Fix/Revert chains:"
git log --oneline | grep -iE "^[0-9a-f]{7} (fix|revert|undo)" | head -10 || echo "    None found"

echo
echo "  Potential Claude Code commits:"
git log --oneline | grep -iE "claude|implement|complete|compile|works|ready|done" | head -10 || echo "    None found"

# ================================================================
# FIND FILES WITH MOST CHANGES (churned files)
# ================================================================
echo
echo -e "${YELLOW}[3/6]${NC} Files with excessive changes (possible issues)..."
echo

git log --all -M -C --name-only --format='format:' | \
    grep -v '^$' | sort | uniq -c | sort -nr | head -10 | \
    while read count file; do
        if [ "$count" -gt 20 ]; then
            echo -e "  ${RED}⚠️${NC}  $file: changed $count times"
        else
            echo "  $file: changed $count times"
        fi
    done

# ================================================================
# FIND BINARY FILES IN HISTORY
# ================================================================
echo
echo -e "${YELLOW}[4/6]${NC} Binary files in git history..."
echo

git log --all --numstat | grep -E "^-\t-\t" | cut -f3 | sort -u | while read file; do
    size=$(git ls-tree -r -l HEAD "$file" 2>/dev/null | awk '{print $4}')
    if [ -n "$size" ] && [ "$size" -gt 100000 ]; then
        echo -e "  ${RED}⚠️${NC}  $file ($(numfmt --to=iec-i --suffix=B $size))"
    elif [ -n "$size" ]; then
        echo "  $file ($(numfmt --to=iec-i --suffix=B $size))"
    fi
done | head -10

# ================================================================
# FIND SENSITIVE PATTERNS
# ================================================================
echo
echo -e "${YELLOW}[5/6]${NC} Searching for sensitive information..."
echo

echo "  Potential secrets in commit messages:"
git log --grep="password\|secret\|token\|key\|credential" --oneline | head -5 || echo "    None found"

echo
echo "  Files with suspicious names:"
git log --all --full-history -- "*secret*" "*password*" "*credential*" "*token*" "*.key" "*.pem" --oneline | head -5 || echo "    None found"

# ================================================================
# ANALYZE AUTHOR PATTERNS
# ================================================================
echo
echo -e "${YELLOW}[6/6]${NC} Commit author analysis..."
echo

echo "  Commits by author:"
git shortlog -sn | head -10

echo
echo "  Recent activity (last 50 commits):"
git log --format='%h %an: %s' -50 | \
    awk '{author=$2; gsub(":", "", author); authors[author]++} 
         END {for (a in authors) printf "    %s: %d commits\n", a, authors[a]}' | \
    sort -k2 -nr

# ================================================================
# RECOMMENDATIONS
# ================================================================
echo
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}                    RECOMMENDATIONS${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo

echo -e "${MAGENTA}Based on the analysis:${NC}"
echo

# Count problems
LARGE_COMMITS=$(git log --oneline --numstat | awk '/^[0-9a-f]{7}/ {if (a>10000 || d>10000) count++; commit=$1; a=0; d=0} /^[0-9]+\t[0-9]+/ {a+=$1; d+=$2} END {print count+0}')
WIP_COMMITS=$(git log --oneline | grep -ciE "WIP|test|tmp|TODO" || echo 0)

if [ "$LARGE_COMMITS" -gt 0 ]; then
    echo -e "  ${RED}●${NC} Found $LARGE_COMMITS large commits (likely monoliths) - SQUASH/DROP"
fi

if [ "$WIP_COMMITS" -gt 0 ]; then
    echo -e "  ${YELLOW}●${NC} Found $WIP_COMMITS WIP/test commits - SQUASH"
fi

echo
echo -e "${GREEN}Suggested cleanup approach:${NC}"
echo
echo "  1. Create backup:"
echo "     ${BLUE}git branch backup/$(date +%Y%m%d)-pre-cleanup${NC}"
echo
echo "  2. Interactive rebase to clean history:"
echo "     ${BLUE}git rebase -i --root${NC}"
echo
echo "  3. For each problematic commit:"
echo "     - ${RED}drop${NC}    → Remove entirely (monoliths, broken code)"
echo "     - ${YELLOW}squash${NC}  → Combine with previous (fixes, WIPs)"
echo "     - ${GREEN}reword${NC}  → Fix commit message"
echo "     - ${GREEN}pick${NC}    → Keep as-is (good commits)"
echo
echo "  4. After rebase, force push to backup:"
echo "     ${BLUE}git push backup --force${NC}"
echo
echo "  5. Final verification:"
echo "     ${BLUE}git log --oneline --graph${NC}"

# ================================================================
# DANGER ZONE
# ================================================================
echo
echo -e "${RED}═══════════════════════════════════════════════════════${NC}"
echo -e "${RED}                    ⚠️  DANGER ZONE ⚠️${NC}"
echo -e "${RED}═══════════════════════════════════════════════════════${NC}"
echo

echo "Before pushing to public, verify:"
echo "  □ No commits with >10K line changes"
echo "  □ No WIP/TODO/test commit messages"
echo "  □ No binary files >1MB"
echo "  □ No files with 'secret/password/token' in name"
echo "  □ No excessive fix/revert chains"
echo "  □ Professional commit messages throughout"
echo "  □ Logical progression of features"

echo
echo -e "${YELLOW}To see the full commit graph:${NC}"
echo "  git log --graph --pretty=format:'%Cred%h%Creset -%C(yellow)%d%Creset %s %Cgreen(%cr) %C(bold blue)<%an>%Creset' --abbrev-commit"

echo
echo -e "${GREEN}✨ Analysis complete!${NC}"
