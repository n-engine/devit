//! # DevIt API de Sérialisation
//!
//! Formats de réponse standardisés et mapping des erreurs pour l'API JSON.
//! Fournit des structures cohérentes pour toutes les réponses de l'API.

use uuid::Uuid;

use super::errors::DevItError::{self, Io};
pub use devit_common::{StdError, StdResponse};

pub fn std_error_from_devit_error(devit_error: DevItError) -> StdError {
    let (code, message, hint, actionable, details) = map_devit_error_to_std_error(&devit_error);

    let mut error = StdError::new(code, message);
    if let Some(h) = hint {
        error = error.with_hint(h);
    }
    if let Some(flag) = actionable {
        error = error.with_actionable(flag);
    }
    if let Some(payload) = details {
        error = error.with_details(payload);
    }
    error
}

/// Mappe un DevItError vers les composants d'un StdError.
///
/// # Arguments
/// * `error` - Erreur DevIt à mapper
///
/// # Returns
/// Tuple (code, message, hint, actionable, details)
pub fn map_devit_error_to_std_error(
    error: &DevItError,
) -> (
    String,
    String,
    Option<String>,
    Option<bool>,
    Option<serde_json::Value>,
) {
    match error {
        DevItError::InvalidDiff {
            reason,
            line_number,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "reason".to_string(),
                serde_json::Value::String(reason.clone()),
            );
            if let Some(line) = line_number {
                details.insert(
                    "line_number".to_string(),
                    serde_json::Value::Number((*line).into()),
                );
            }

            (
                "E_INVALID_DIFF".to_string(),
                "Format de patch invalide ou corrompu".to_string(),
                Some("Vérifiez que le patch est un diff unifié valide".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::SnapshotRequired {
            operation,
            expected,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "operation".to_string(),
                serde_json::Value::String(operation.clone()),
            );
            details.insert(
                "expected".to_string(),
                serde_json::Value::String(expected.clone()),
            );

            (
                "E_SNAPSHOT_REQUIRED".to_string(),
                "Un snapshot valide est requis pour cette opération".to_string(),
                Some("Créez un snapshot avant de continuer".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::SnapshotStale {
            snapshot_id,
            created_at,
            staleness_reason,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "snapshot_id".to_string(),
                serde_json::Value::String(snapshot_id.clone()),
            );
            if let Some(timestamp) = created_at {
                details.insert(
                    "created_at".to_string(),
                    serde_json::Value::String(timestamp.to_rfc3339()),
                );
            }
            if let Some(reason) = staleness_reason {
                details.insert(
                    "staleness_reason".to_string(),
                    serde_json::Value::String(reason.clone()),
                );
            }

            (
                "E_SNAPSHOT_STALE".to_string(),
                "Le snapshot est obsolète par rapport à l'état actuel".to_string(),
                Some("Créez un nouveau snapshot ou validez les changements".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::PolicyBlock {
            rule,
            required_level,
            current_level,
            context,
        } => {
            let mut details = serde_json::Map::new();
            details.insert("rule".to_string(), serde_json::Value::String(rule.clone()));
            details.insert(
                "required_level".to_string(),
                serde_json::Value::String(required_level.clone()),
            );
            details.insert(
                "current_level".to_string(),
                serde_json::Value::String(current_level.clone()),
            );
            details.insert(
                "context".to_string(),
                serde_json::Value::String(context.clone()),
            );

            (
                "E_POLICY_BLOCK".to_string(),
                "La politique de sécurité bloque l'opération demandée".to_string(),
                Some("Augmentez le niveau d'approbation ou modifiez la politique".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::ProtectedPath {
            path,
            protection_rule,
            attempted_operation,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "path".to_string(),
                serde_json::Value::String(path.to_string_lossy().to_string()),
            );
            details.insert(
                "protection_rule".to_string(),
                serde_json::Value::String(protection_rule.clone()),
            );
            details.insert(
                "attempted_operation".to_string(),
                serde_json::Value::String(attempted_operation.clone()),
            );

            (
                "E_PROTECTED_PATH".to_string(),
                "L'opération affecte un fichier ou répertoire protégé".to_string(),
                Some(
                    "Utilisez un niveau d'approbation plus élevé ou excluez ce chemin".to_string(),
                ),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::PrivilegeEscalation {
            escalation_type,
            current_privileges,
            attempted_privileges,
            security_context,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "escalation_type".to_string(),
                serde_json::Value::String(escalation_type.clone()),
            );
            details.insert(
                "current_privileges".to_string(),
                serde_json::Value::String(current_privileges.clone()),
            );
            details.insert(
                "attempted_privileges".to_string(),
                serde_json::Value::String(attempted_privileges.clone()),
            );
            details.insert(
                "security_context".to_string(),
                serde_json::Value::String(security_context.clone()),
            );

            (
                "E_PRIV_ESCALATION".to_string(),
                "L'opération tente une escalade de privilèges".to_string(),
                None,
                Some(false),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::GitDirty {
            dirty_files,
            modified_files,
            branch,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "dirty_files".to_string(),
                serde_json::Value::Number((*dirty_files).into()),
            );
            let files: Vec<serde_json::Value> = modified_files
                .iter()
                .map(|p| serde_json::Value::String(p.to_string_lossy().to_string()))
                .collect();
            details.insert(
                "modified_files".to_string(),
                serde_json::Value::Array(files),
            );
            if let Some(branch_name) = branch {
                details.insert(
                    "branch".to_string(),
                    serde_json::Value::String(branch_name.clone()),
                );
            }

            (
                "E_GIT_DIRTY".to_string(),
                "Le répertoire de travail Git a des changements non commités".to_string(),
                Some("Commitez ou stashez les changements avant de continuer".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::VcsConflict {
            location,
            conflict_type,
            conflicted_files,
            resolution_hint,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "location".to_string(),
                serde_json::Value::String(location.clone()),
            );
            details.insert(
                "conflict_type".to_string(),
                serde_json::Value::String(conflict_type.clone()),
            );
            let files: Vec<serde_json::Value> = conflicted_files
                .iter()
                .map(|p| serde_json::Value::String(p.to_string_lossy().to_string()))
                .collect();
            details.insert(
                "conflicted_files".to_string(),
                serde_json::Value::Array(files),
            );
            if let Some(hint) = resolution_hint {
                details.insert(
                    "resolution_hint".to_string(),
                    serde_json::Value::String(hint.clone()),
                );
            }

            (
                "E_VCS_CONFLICT".to_string(),
                "Conflit détecté dans le système de contrôle de version".to_string(),
                resolution_hint.clone(),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::TestFail {
            failed_count,
            total_count,
            test_framework,
            failure_details,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "failed_count".to_string(),
                serde_json::Value::Number((*failed_count).into()),
            );
            details.insert(
                "total_count".to_string(),
                serde_json::Value::Number((*total_count).into()),
            );
            details.insert(
                "test_framework".to_string(),
                serde_json::Value::String(test_framework.clone()),
            );
            let failure_array: Vec<serde_json::Value> = failure_details
                .iter()
                .map(|f| serde_json::Value::String(f.clone()))
                .collect();
            details.insert(
                "failure_details".to_string(),
                serde_json::Value::Array(failure_array),
            );

            (
                "E_TEST_FAIL".to_string(),
                "L'exécution des tests a échoué".to_string(),
                Some("Corrigez les tests qui échouent et réessayez".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::TestTimeout {
            timeout_secs,
            test_framework,
            running_tests,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "timeout_secs".to_string(),
                serde_json::Value::Number((*timeout_secs).into()),
            );
            details.insert(
                "test_framework".to_string(),
                serde_json::Value::String(test_framework.clone()),
            );
            let tests_array: Vec<serde_json::Value> = running_tests
                .iter()
                .map(|t| serde_json::Value::String(t.clone()))
                .collect();
            details.insert(
                "running_tests".to_string(),
                serde_json::Value::Array(tests_array),
            );

            (
                "E_TEST_TIMEOUT".to_string(),
                "L'exécution des tests a dépassé la limite de temps".to_string(),
                Some("Augmentez le timeout ou optimisez les tests".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::SandboxDenied {
            reason,
            active_profile,
            attempted_operation,
            violated_policy,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "reason".to_string(),
                serde_json::Value::String(reason.clone()),
            );
            details.insert(
                "active_profile".to_string(),
                serde_json::Value::String(active_profile.clone()),
            );
            details.insert(
                "attempted_operation".to_string(),
                serde_json::Value::String(attempted_operation.clone()),
            );
            if let Some(policy) = violated_policy {
                details.insert(
                    "violated_policy".to_string(),
                    serde_json::Value::String(policy.clone()),
                );
            }

            (
                "E_SANDBOX_DENIED".to_string(),
                "La politique de sandbox a refusé l'opération".to_string(),
                Some("Utilisez un profil de sandbox moins restrictif".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::ResourceLimit {
            resource_type,
            current_usage,
            limit,
            unit,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "resource_type".to_string(),
                serde_json::Value::String(resource_type.clone()),
            );
            details.insert(
                "current_usage".to_string(),
                serde_json::Value::Number((*current_usage).into()),
            );
            details.insert(
                "limit".to_string(),
                serde_json::Value::Number((*limit).into()),
            );
            details.insert("unit".to_string(), serde_json::Value::String(unit.clone()));

            (
                "E_RESOURCE_LIMIT".to_string(),
                "Limite de ressource système dépassée".to_string(),
                Some("Augmentez les limites ou optimisez l'utilisation des ressources".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        Io {
            operation,
            path,
            source,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "operation".to_string(),
                serde_json::Value::String(operation.clone()),
            );
            if let Some(p) = path {
                details.insert(
                    "path".to_string(),
                    serde_json::Value::String(p.to_string_lossy().to_string()),
                );
            }
            details.insert(
                "source".to_string(),
                serde_json::Value::String(source.to_string()),
            );

            let is_permission_error = source.kind() == std::io::ErrorKind::PermissionDenied;
            let hint = if is_permission_error {
                Some("Vérifiez les permissions d'accès au fichier".to_string())
            } else {
                Some("Vérifiez que le fichier existe et est accessible".to_string())
            };

            (
                "E_IO".to_string(),
                "Erreur d'entrée/sortie lors de l'opération".to_string(),
                hint,
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::Internal {
            component,
            message,
            cause,
            correlation_id,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "component".to_string(),
                serde_json::Value::String(component.clone()),
            );
            details.insert(
                "message".to_string(),
                serde_json::Value::String(message.clone()),
            );
            if let Some(cause_desc) = cause {
                details.insert(
                    "cause".to_string(),
                    serde_json::Value::String(cause_desc.clone()),
                );
            }
            details.insert(
                "correlation_id".to_string(),
                serde_json::Value::String(correlation_id.clone()),
            );

            (
                "E_INTERNAL".to_string(),
                "Erreur interne ou condition inattendue".to_string(),
                Some("Contactez le support technique avec les détails de l'erreur".to_string()),
                Some(false),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::InvalidTestConfig {
            field,
            value,
            reason,
        } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "field".to_string(),
                serde_json::Value::String(field.clone()),
            );
            details.insert(
                "value".to_string(),
                serde_json::Value::String(value.clone()),
            );
            details.insert(
                "reason".to_string(),
                serde_json::Value::String(reason.clone()),
            );

            (
                "E_INVALID_TEST_CONFIG".to_string(),
                "Configuration de test invalide".to_string(),
                Some("Vérifiez les paramètres de configuration des tests".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }

        DevItError::InvalidFormat { format, supported } => {
            let mut details = serde_json::Map::new();
            details.insert(
                "format".to_string(),
                serde_json::Value::String(format.clone()),
            );
            details.insert(
                "supported".to_string(),
                serde_json::Value::Array(
                    supported
                        .iter()
                        .map(|s| serde_json::Value::String(s.clone()))
                        .collect(),
                ),
            );

            (
                "E_INVALID_FORMAT".to_string(),
                "Format de sortie non supporté".to_string(),
                Some("Utilisez un des formats supportés".to_string()),
                Some(true),
                Some(serde_json::Value::Object(details)),
            )
        }
    }
}

/// Fonctions utilitaires pour créer des réponses standardisées courantes.
pub mod responses {
    use super::*;

    /// Crée une réponse de succès simple sans données.
    ///
    /// # Arguments
    /// * `request_id` - Identifiant optionnel de la requête
    ///
    /// # Returns
    /// Réponse de succès avec data = ()
    pub fn success_empty(request_id: Option<Uuid>) -> StdResponse<()> {
        StdResponse::success((), request_id)
    }

    /// Crée une réponse d'erreur pour une validation échouée.
    ///
    /// # Arguments
    /// * `message` - Message d'erreur
    /// * `request_id` - Identifiant optionnel de la requête
    ///
    /// # Returns
    /// Réponse d'erreur de validation
    pub fn validation_error(message: String, request_id: Option<Uuid>) -> StdResponse<()> {
        let error = StdError::new("E_VALIDATION".to_string(), message).with_actionable(true);
        StdResponse::error(error, request_id)
    }

    /// Crée une réponse d'erreur pour une requête malformée.
    ///
    /// # Arguments
    /// * `details` - Détails sur l'erreur de format
    /// * `request_id` - Identifiant optionnel de la requête
    ///
    /// # Returns
    /// Réponse d'erreur de format
    pub fn malformed_request(details: String, request_id: Option<Uuid>) -> StdResponse<()> {
        let error = StdError::new(
            "E_MALFORMED_REQUEST".to_string(),
            "Requête malformée".to_string(),
        )
        .with_hint("Vérifiez le format JSON de votre requête".to_string())
        .with_actionable(true)
        .with_details(serde_json::json!({ "details": details }));

        StdResponse::error(error, request_id)
    }
}
