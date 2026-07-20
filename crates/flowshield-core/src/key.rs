use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key(Arc<str>);

impl Key {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Key(Arc::from(value))
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Key(Arc::from(value))
    }
}

/// Extracts a key from some context `C` (e.g. an HTTP request).
/// Defined in core as a stable seam; concrete HTTP extractors live in the
/// integration crates (Phase 5).
pub trait KeyExtractor<C>: Send + Sync {
    fn extract_key(&self, ctx: &C) -> Key;
}
