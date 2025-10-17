use once_cell::sync::Lazy;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Tracks request identifiers for idempotent calls within the current process.
static REQUEST_TRACKER: Lazy<Mutex<HashMap<Uuid, Uuid>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Resolves a `request_id` for the given idempotency key.
///
/// - When a key is provided and already known, the previously generated
///   request identifier is returned.
/// - Otherwise a new UUIDv4 is generated.
pub fn resolve(idempotency_key: Option<Uuid>) -> Uuid {
    if let Some(key) = idempotency_key {
        let mut guard = REQUEST_TRACKER.lock().expect("request tracker poisoned");
        match guard.entry(key) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(vacant) => {
                let request_id = Uuid::new_v4();
                vacant.insert(request_id);
                request_id
            }
        }
    } else {
        Uuid::new_v4()
    }
}

#[cfg(test)]
pub fn reset_for_tests() {
    REQUEST_TRACKER
        .lock()
        .expect("request tracker poisoned")
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_returns_same_id_for_same_key() {
        reset_for_tests();
        let key = Uuid::new_v4();
        let first = resolve(Some(key));
        let second = resolve(Some(key));
        assert_eq!(first, second);
    }

    #[test]
    fn resolve_generates_new_ids_for_none() {
        reset_for_tests();
        let first = resolve(None);
        let second = resolve(None);
        assert_ne!(first, second);
    }
}
