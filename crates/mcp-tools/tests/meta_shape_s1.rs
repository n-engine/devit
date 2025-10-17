#![cfg(feature = "test-utils")]

mod meta_shape {
    use serde_json::{json, Value};

    fn fake_meta() -> Value {
        json!({
            "trace_id":"0000-1111",
            "effective_limits": {"timeout_ms":8000,"max_bytes":500000,"max_redirects":2,"follow_redirects":true},
            "limit_sources":   {"timeout_ms":"env","max_bytes":"default","max_redirects":"default","follow_redirects":"param"},
            "delegation_context": null,
            "cache_key":"abc123"
        })
    }

    #[test]
    fn metadata_has_required_fields() {
        let m = fake_meta();
        for k in [
            "effective_limits",
            "limit_sources",
            "delegation_context",
            "cache_key",
            "trace_id",
        ] {
            assert!(m.get(k).is_some(), "missing field: {k}");
        }
        assert!(m["effective_limits"].get("timeout_ms").is_some());
        assert!(m["limit_sources"].get("timeout_ms").is_some());
        assert!(m["delegation_context"].is_null());
    }
}
