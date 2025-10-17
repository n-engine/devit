# LLM Pseudo-Patch Corpus

Collection d'exemples de pseudo-patches générés par des LLMs, utilisés pour les tests de non-régression du `devit-patch-fix`.

## Formats d'entrée

### 01_begin_end_format.patch
Format "Begin/End Patch" typique des LLMs freestyle
- **Taux de succès attendu**: ≥95%
- **Format**: Begin/End délimiteurs

### 02_incomplete_git_diff.patch
Diff git-like avec headers incomplets
- **Problème**: Manque counts dans @@ header
- **Normalisation**: Inférence automatique des counts

### 03_crlf_windows_paths.patch
Chemins Windows avec backslashes
- **Problème**: Paths Windows vs Unix
- **Normalisation**: Conversion automatique des paths

### 04_raw_changes.patch
Changements bruts sans contexte
- **Problème**: Pas de headers ni contexte
- **Normalisation**: Génération de contexte via file search

### 05_unicode_whitespace.patch
Espaces Unicode et formatage approximatif
- **Problème**: NBSP, tabs, espaces non-standards
- **Normalisation**: Unicode normalization

## Utilisation des Tests

```bash
# Test du corpus complet
for patch in docs/examples/patches/*.patch; do
  echo "Testing: $patch"
  cargo run -p mcp-server -- --working-dir . -- test-patch-fix --file "$patch"
done
```

## Métriques Cibles

- **fix_success_rate**: ≥95% sur ce corpus
- **apply_success_rate**: ≥90% (le reste = vrais conflits)
- **latency_p95**: < 20ms par patch

## Expansion du Corpus

Ajouter de nouveaux exemples issus de :
- GPT-4/ChatGPT approximations
- Claude/Mistral/Qwen variations
- LLaMA freestyle outputs
- Edge cases découverts en production
