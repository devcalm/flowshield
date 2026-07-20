#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Config(String),

    #[error("storage backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("backend operation timed out after {elapsed:?}")]
    Timeout { elapsed: std::time::Duration },
}

pub type Result<T> = std::result::Result<T, Error>;
