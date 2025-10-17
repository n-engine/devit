use sha2::{Digest, Sha256};

/// Stable cache key for web search/fetch responses.
/// Includes parameters that materially affect the payload shape/permissions.
pub fn cache_key(
    url_or_query: &str,
    accept: &str,
    user_agent: &str,
    safe_mode: &str,
    include_content: bool,
) -> String {
    let material = format!(
        "{}\n{}\n{}\n{}\n{}",
        url_or_query.to_lowercase(),
        accept,
        user_agent,
        safe_mode,
        include_content
    );
    format!("{:x}", Sha256::digest(material.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::cache_key;

    #[test]
    fn cache_key_differs_by_safe_mode() {
        let k1 = cache_key("https://ex", "text/html", "DevItBot/1.0", "strict", true);
        let k2 = cache_key("https://ex", "text/html", "DevItBot/1.0", "off", true);
        assert_ne!(k1, k2);
    }
}
