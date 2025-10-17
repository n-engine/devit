use std::collections::HashSet;

use axum::http::HeaderValue;

#[derive(Clone)]
pub(crate) struct AuthManager {
    tokens: HashSet<String>,
}

impl AuthManager {
    pub(crate) fn new(tokens: HashSet<String>) -> Self {
        Self { tokens }
    }

    pub(crate) fn validate(&self, header: Option<&HeaderValue>) -> bool {
        if self.tokens.is_empty() {
            return true;
        }

        let raw = match header.and_then(|value| value.to_str().ok()) {
            Some(value) => value.trim(),
            None => return false,
        };

        let token = raw
            .strip_prefix("Bearer ")
            .or_else(|| raw.strip_prefix("bearer "))
            .unwrap_or(raw)
            .trim();

        self.tokens.contains(token)
    }
}
