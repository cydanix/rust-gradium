//! Error types for the Gradium client library.

use thiserror::Error;

/// Error type for Gradium client operations.
#[derive(Error, Debug)]
pub enum Error {
    /// WebSocket connection error.
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Client not ready for operations.
    #[error("Client not ready")]
    NotReady,

    /// Client is stopping.
    #[error("Client is stopping")]
    Stopping,

    /// Queue is full.
    #[error("Queue is full")]
    QueueFull,

    /// Connection timeout.
    #[error("Connection timeout")]
    ConnectionTimeout,

    /// Ready timeout - client did not become ready in time.
    #[error("Ready timeout")]
    ReadyTimeout,

    /// Server returned an error.
    #[error("Server error: {message} (code: {code})")]
    ServerError {
        /// Error message from server.
        message: String,
        /// Error code from server.
        code: i32,
    },

    /// Unknown message type received.
    #[error("Unknown message type: {0}")]
    UnknownMessageType(String),

    /// Channel send error.
    #[error("Channel send error")]
    ChannelSend,

    /// Channel receive error.
    #[error("Channel receive error")]
    ChannelRecv,
}

