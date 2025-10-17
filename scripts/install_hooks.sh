#!/bin/bash
# Installation des Git Hooks DevIt - Anti-Stub System
set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}╔════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║   🛡️  DevIt Git Hooks Installation    ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════╝${NC}"
echo ""

# Vérifier qu'on est dans un repo git
if [ ! -d .git ]; then
    echo -e "${RED}❌ Erreur: Pas de dossier .git trouvé!${NC}"
    echo "   Lancez ce script depuis la racine du projet DevIt"
    exit 1
fi

# Créer le dossier hooks si nécessaire
mkdir -p .git/hooks

# Copier le pre-commit hook
echo -e "${YELLOW}📋 Installation du hook pre-commit...${NC}"
cp git-hooks/pre-commit .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit

# Test rapide du hook
echo -e "${YELLOW}🧪 Test du hook...${NC}"
if .git/hooks/pre-commit 2>/dev/null; then
    echo -e "${GREEN}  ✅ Hook fonctionnel!${NC}"
else
    echo -e "${YELLOW}  ⚠️  Hook installé (test skippé car pas de changements)${NC}"
fi

echo ""
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo -e "${GREEN}✅ Installation complète!${NC}"
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo ""
echo -e "${YELLOW}📋 Le hook vérifie automatiquement:${NC}"
echo "  • Pas de unimplemented!()"
echo "  • Pas de todo!()"
echo "  • Pas de TODO/FIXME dans les commentaires"
echo "  • Pas de return Ok(()) vides suspects"
echo "  • Pas de panic! non justifiés"
echo "  • Pas de tests #[ignore]"
echo ""
echo -e "${YELLOW}🔧 Commandes utiles:${NC}"
echo "  • Tester: git-hooks/pre-commit"
echo "  • Bypass: git commit --no-verify (URGENCE SEULEMENT!)"
echo "  • Désactiver: rm .git/hooks/pre-commit"
echo ""
echo -e "${GREEN}💪 Claude Code ne pourra plus commit de stubs!${NC}"
