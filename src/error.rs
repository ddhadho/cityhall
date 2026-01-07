use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Corrupted data: {0}")]
    Corruption(String),

    #[error("Corrupted data: {0}")]
    CorruptedData(String),

    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Key not found")]
    KeyNotFound,

    #[error("Channel send error: {0}")]
    ChannelSend(String),

    #[error("System time error: {0}")]
    SystemTime(String),
}

// Conversion for channel errors
impl<T> From<crossbeam_channel::SendError<T>> for StorageError {
    fn from(err: crossbeam_channel::SendError<T>) -> Self {
        StorageError::ChannelSend(err.to_string())
    }
}

// Conversion for system time errors
impl From<std::time::SystemTimeError> for StorageError {
    fn from(err: std::time::SystemTimeError) -> Self {
        StorageError::SystemTime(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;
