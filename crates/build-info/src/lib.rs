use once_cell::sync::Lazy;

#[derive(Debug)]
struct BuildMeta {
    build_id: String,
    build_time: String,
    git_label: String,
}

impl BuildMeta {
    fn collect() -> Self {
        let build_id = option_env!("DEVIT_BUILD_ID")
            .unwrap_or("unknown build")
            .to_string();
        let build_time = option_env!("DEVIT_BUILD_TIME")
            .unwrap_or("unknown time")
            .to_string();
        let git_label = option_env!("DEVIT_BUILD_GIT")
            .unwrap_or("unknown git")
            .to_string();
        Self {
            build_id,
            build_time,
            git_label,
        }
    }
}

static META: Lazy<BuildMeta> = Lazy::new(BuildMeta::collect);

/// Identifiant complet de build (ex: "2025-10-05 15:47:12 UTC | v1.2.3-8a4f1d2-dirty").
pub fn build_id() -> &'static str {
    META.build_id.as_str()
}

/// Timestamp lisible (UTC) généré au build.
pub fn build_timestamp() -> &'static str {
    META.build_time.as_str()
}

/// Label git (tag/commit) détecté au build.
pub fn git_label() -> &'static str {
    META.git_label.as_str()
}

/// Format prêt à afficher pour un binaire spécifique.
pub fn formatted_banner(package: &str, version: &str) -> String {
    format!("{} {} | {}", package, version, build_id())
}
