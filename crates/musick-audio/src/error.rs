use thiserror::Error;

pub type AudioResult<T> = Result<T, AudioError>;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Audio backend error: {0}")]
    Backend(String),

    #[error("Stream resolver error: {0}")]
    Resolver(String),

    #[error("Core error: {0}")]
    Core(#[from] musick_core::CoreError),
}
