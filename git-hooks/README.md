# 🛡️ DevIt Git Hooks - Protection Anti-Stubs

## Le Problème

Claude Code (et autres AI) ont tendance à produire du code avec des stubs :
- `unimplemented!()`
- `todo!()`
- `return Ok(())` vides
- Commentaires `// TODO` partout
- Tests ignorés avec `#[ignore]`

**C'est INACCEPTABLE pour un projet de production !**

## La Solution

Ce système de git hooks **BLOQUE PHYSIQUEMENT** les commits contenant des stubs.

### Installation

```bash
# Depuis la racine du projet
./scripts/install_hooks.sh
```

### Ce que ça vérifie

Le hook `pre-commit` scanne TOUS les fichiers modifiés et refuse le commit si il trouve :

| Pattern | Exemple | Action |
|---------|---------|--------|
| `unimplemented!()` | Fonction non implémentée | ❌ BLOQUÉ |
| `todo!()` | Macro todo Rust | ❌ BLOQUÉ |
| `TODO:` ou `FIXME:` | Commentaires de travail | ❌ BLOQUÉ |
| `return Ok(())` | Return vide suspect | ❌ BLOQUÉ |
| `panic!("not implemented")` | Panic de stub | ❌ BLOQUÉ |
| `#[ignore]` | Tests désactivés | ❌ BLOQUÉ |

### Exemple de blocage

```
🔍 DevIt Pre-Commit: Vérification anti-stubs...
❌ COMMIT BLOQUÉ - Code stub détecté!
═══════════════════════════════════════
  ⚠️  src/daemon.rs contient: unimplemented!()
  ⚠️  src/backend.rs contient: TODO:
═══════════════════════════════════════
📝 Instructions pour Claude Code:
  1. Remplacez TOUS les stubs par du vrai code
  2. unimplemented!() → implémentez la logique complète
  3. return Ok(()) → retournez une vraie valeur avec données
  4. TODO/FIXME → finissez le travail MAINTENANT

⚡ DIRECTIVE CLAUDE CODE:
  Vous DEVEZ implémenter le code complet.
  Pas de placeholder, pas de stub, du VRAI CODE!
```

### Bypass d'urgence

**UNIQUEMENT en cas d'urgence absolue** :

```bash
git commit --no-verify -m "URGENT: bypass hook pour raison X"
```

⚠️ **À utiliser avec parcimonie !** Tout bypass doit être justifié.

### Pour Claude Code

**DIRECTIVE ABSOLUE** : Vous devez toujours implémenter le code COMPLÈTEMENT.
- Pas de `unimplemented!()`
- Pas de `todo!()`
- Pas de "je finirai plus tard"
- Du code qui COMPILE et qui FONCTIONNE

Si vous ne savez pas comment implémenter quelque chose, DEMANDEZ plutôt que de mettre un stub !

## Structure

```
git-hooks/
├── pre-commit      # Le hook qui vérifie les stubs
└── README.md       # Cette doc

scripts/
└── install_hooks.sh  # Script d'installation automatique
```

## Maintenance

Pour mettre à jour les patterns bloqués, éditez `git-hooks/pre-commit` et ajoutez/modifiez dans le tableau `FORBIDDEN_PATTERNS`.

---

**Remember: No stubs, no excuses, only working code!** 💪
