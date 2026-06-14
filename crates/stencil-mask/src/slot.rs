//! Slot derivation: value → `Slot` (id + description).
//!
//! The id is either a name (when the value matches a resolved entry
//! in values.toml) or `sha256(value)[:8]`. The description comes from
//! the describer pipeline.

use std::collections::HashMap;

use sha2::{Digest, Sha256};
use stencil_core::{Slot, SlotId};

use crate::describer::describe;

/// In-memory map from real value (bytes) to user-given name.
/// Built by the daemon from values.toml references that are safe to
/// resolve for passive masking.
#[derive(Debug, Default, Clone)]
pub struct ValueNameMap {
    map: HashMap<String, String>,
}

impl ValueNameMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, value: String, name: String) {
        self.map.insert(value, name);
    }

    pub fn lookup(&self, value: &str) -> Option<&str> {
        self.map.get(value).map(String::as_str)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map
            .iter()
            .map(|(value, name)| (value.as_str(), name.as_str()))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Derive a slot for a value. Pure function aside from the
/// [`ValueNameMap`] lookup.
pub fn derive_slot(value: &str, vn: &ValueNameMap) -> Slot {
    let id = match vn.lookup(value) {
        Some(name) => SlotId::Named(name.to_string()),
        None => SlotId::Hash(short_hash(value)),
    };
    Slot {
        id,
        description: describe(value),
    }
}

fn short_hash(value: &str) -> String {
    let mut h = Sha256::new();
    h.update(value.as_bytes());
    let digest = h.finalize();
    hex::encode(&digest[..4]) // 4 bytes → 8 hex chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unnamed_uses_hash() {
        let vn = ValueNameMap::new();
        let s = derive_slot("12345678", &vn);
        assert!(
            matches!(s.id, SlotId::Hash(ref h) if h.len() == 8 && h.chars().all(|c| c.is_ascii_hexdigit()))
        );
        assert_eq!(s.description, "8-digit numeric");
    }

    #[test]
    fn named_uses_name() {
        let mut vn = ValueNameMap::new();
        vn.insert("12345678".into(), "ira_account".into());
        let s = derive_slot("12345678", &vn);
        assert!(matches!(s.id, SlotId::Named(ref n) if n == "ira_account"));
        assert_eq!(s.description, "8-digit numeric");
    }

    #[test]
    fn render_format() {
        let mut vn = ValueNameMap::new();
        vn.insert("12345678".into(), "ira_account".into());
        let s = derive_slot("12345678", &vn);
        assert_eq!(s.render(), "[$ira_account 8-digit numeric]");
    }

    #[test]
    fn same_value_same_slot() {
        let vn = ValueNameMap::new();
        let a = derive_slot("203.0.113.7", &vn);
        let b = derive_slot("203.0.113.7", &vn);
        assert_eq!(a, b);
    }

    #[test]
    fn real_value_never_in_slot_string() {
        let vn = ValueNameMap::new();
        let secret = "87654321";
        let s = derive_slot(secret, &vn);
        let rendered = s.render();
        assert!(
            !rendered.contains(secret),
            "real value leaked into slot rendering: {rendered}"
        );
    }
}
