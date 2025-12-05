//! Text-to-Speech client for the Gradium API.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info};

use crate::error::Error;
use crate::messages::*;
use crate::ws::WebSocket;

/// Events emitted by the TTS client.
#[derive(Debug, Clone)]
pub enum TtsEvent {
    /// The TTS client is ready.
    Ready {
        /// Request ID for this session.
        request_id: String,
    },
    /// Audio chunk received (base64-encoded PCM).
    Audio {
        /// Base64-encoded audio data.
        audio: String,
    },
    /// Text echo from server.
    TextEcho {
        /// The text that was sent.
        text: String,
    },
    /// An error occurred.
    Error {
        /// Error message.
        message: String,
        /// Error code.
        code: i32,
    },
    /// End of stream.
    EndOfStream,

    Ping,
    Pong,
    Close,
    Frame,
}

/// Configuration for the TTS client.
#[derive(Debug, Clone)]
pub struct TtsConfig {
    /// WebSocket endpoint URL.
    pub endpoint: String,
    /// Voice ID to use for synthesis.
    pub voice_id: String,
    /// API key for authentication.
    pub api_key: String,
    /// Model name (default: "default").
    pub model_name: String,
    /// Output format (default: "pcm").
    pub output_format: String,
}

impl TtsConfig {
    /// Creates a new TTS configuration with default model and format.
    pub fn new(endpoint: String, voice_id: String, api_key: String) -> Self {
        Self {
            endpoint,
            voice_id,
            api_key,
            model_name: "default".to_string(),
            output_format: "pcm".to_string(),
        }
    }
}

/// Text-to-Speech client for streaming audio synthesis.
pub struct TtsClient {
    config: TtsConfig,
    conn: RwLock<Option<Arc<WebSocket>>>,
    ready: Arc<AtomicBool>,
    error_count: Arc<AtomicU32>,
    stopping: Arc<AtomicBool>,
    session_id: String,
}

impl TtsClient {
    /// Creates a new TTS client with the given configuration.
    ///
    /// Returns the client and an event receiver for receiving TTS events.
    pub fn new(config: TtsConfig) -> Self {
        let client = Self {
            config,
            conn: RwLock::new(None),
            ready: Arc::new(AtomicBool::new(false)),
            error_count: Arc::new(AtomicU32::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            session_id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
        };
        client
    }

    /// Starts the TTS client, connecting to the server and waiting for ready state.
    pub async fn start(&self) -> Result<(), Error> {
        info!(session_id = %self.session_id, "TTS starting");

        // Reset state
        self.ready.store(false, Ordering::SeqCst);
        self.error_count.store(0, Ordering::SeqCst);
        self.stopping.store(false, Ordering::SeqCst);

        // Connect
        let conn = WebSocket::connect(&self.config.endpoint, &self.config.api_key).await?;
        let conn = Arc::new(conn);
        *self.conn.write().await = Some(Arc::clone(&conn));

        // Send setup
        self.send_setup().await?;

        loop {
            let event = self.next_event().await?;
            match event {
                TtsEvent::Ready { request_id } => {
                    info!(request_id = %request_id, "TTS ready");
                    self.ready.store(true, Ordering::SeqCst);
                    break;
                }
                TtsEvent::Error { message, code } => {
                    error!(message = %message, code = code, "TTS error");
                    self.error_count.fetch_add(1, Ordering::SeqCst);
                    return Err(Error::ServerError { message, code });
                }
                _ => {
                    return Err(Error::UnexpectedEventType);
                }
            }
        }

        info!(session_id = %self.session_id, "TTS started");
        Ok(())
    }

    /// Shuts down the TTS client.
    ///
    /// Sends end_of_stream, waits for all audio to be received, then closes the connection.
    pub async fn shutdown(&self) {
        info!(session_id = %self.session_id, "TTS shutting down");

        // Close connection
        if let Some(conn) = self.conn.write().await.take() {
            let _ = conn.close().await;
        }

        info!(session_id = %self.session_id, "TTS shut down");
    }

    /// Sends text for synthesis.
    ///
    /// Returns an error if the client is not ready or is stopping.
    pub async fn process(&self, text: &str) -> Result<(), Error> {
        if !self.is_running() {
            return Err(Error::NotReady);
        }
        self.send_text(text).await
    }

    /// Returns true if the client is ready and running.
    pub fn is_running(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
            && self.error_count.load(Ordering::SeqCst) == 0
            && !self.stopping.load(Ordering::SeqCst)
    }

    /// Returns true if the client is ready.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// Returns the current error count.
    pub fn error_count(&self) -> u32 {
        self.error_count.load(Ordering::SeqCst)
    }

    async fn send_setup(&self) -> Result<(), Error> {
        let payload = TtsSetupMessage::new(
            self.config.voice_id.clone(),
            self.config.model_name.clone(),
            self.config.output_format.clone(),
        );
        let json = serde_json::to_string(&payload)?;
        debug!(json = %json, "Sending TTS setup");
        self.conn.read().await.as_ref().unwrap().send_text(&json).await
    }

    async fn send_text(&self, text: &str) -> Result<(), Error> {
        let payload = TtsTextMessage::new(text.to_string());
        let json = serde_json::to_string(&payload)?;
        debug!(json = %json, "Sending TTS text");
        self.conn.read().await.as_ref().unwrap().send_text(&json).await
    }

    pub async fn send_eos(&self) -> Result<(), Error> {
        debug!("Sending TTS EOS");

        self.stopping.store(true, Ordering::SeqCst);

        let payload = EosMessage::new();
        let json = serde_json::to_string(&payload)?;
        if let Some(conn) = self.conn.read().await.as_ref() {
            conn.send_text(&json).await
        } else {
            Ok(())
        }
    }

    pub async fn next_event(&self) -> Result<TtsEvent, Error> {
        let msg = match self.conn.read().await.as_ref().unwrap().recv().await {
            Ok(msg) => msg,
            Err(e) => return Err(e),
        };

        let text = match &msg {
            Message::Text(t) => {
                //debug!(msg = %t, "TTS received text message");
                t.clone()
            }
            Message::Binary(b) => {
                debug!(len = b.len(), "TTS received binary message");
                match String::from_utf8(b.clone()) {
                    Ok(s) => s,
                    Err(e) => {
                        error!(error = %e, "Invalid UTF-8 in binary message");
                        return Err(Error::InvalidUtf8);
                    }
                }
            }
            Message::Ping(_) => {
                debug!("TTS received ping");
                return Ok(TtsEvent::Ping);
            }
            Message::Pong(_) => {
                debug!("TTS received pong");
                return Ok(TtsEvent::Pong);
            }
            Message::Close(frame) => {
                debug!(frame = ?frame, "TTS received close");
                return Ok(TtsEvent::Close);
            }
            Message::Frame(_) => {
                debug!("TTS received raw frame");
                return Ok(TtsEvent::Frame);
            }
        };

        let generic: GenericMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                error!(error = %e, "Failed to parse message");
                return Err(Error::InvalidJson);
            }
        };

        match generic.msg_type.as_str() {
            "audio" => {
                let msg: TtsAudioMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, "Failed to parse audio message");
                        return Err(Error::InvalidJson);
                    }
                };
                debug!(audio_len = msg.audio.len(), "TTS audio chunk received");
                Ok(TtsEvent::Audio { audio: msg.audio })
            }
            "ready" => {
                let msg: TtsReadyMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, "Failed to parse ready message");
                        return Err(Error::InvalidJson);
                    }
                };
                info!(request_id = %msg.request_id, "TTS ready");
                self.ready.store(true, Ordering::SeqCst);
                Ok(TtsEvent::Ready {request_id: msg.request_id,})
            }
            "text" => {
                let msg: TtsTextMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, "Failed to parse text message");
                        return Err(Error::InvalidJson);
                    }
                };
                debug!(text = %msg.text, "TTS text echo");
                Ok(TtsEvent::TextEcho { text: msg.text })
            }
            "error" => {
                let msg: ErrorMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, "Failed to parse error message");
                        return Err(Error::InvalidJson);
                    }
                };
                error!(message = %msg.message, code = msg.code, "TTS error");
                self.error_count.fetch_add(1, Ordering::SeqCst);
                Ok(TtsEvent::Error {
                    message: msg.message,
                    code: msg.code,
                })
            }
            "end_of_stream" => {
                info!("TTS end of stream");
                Ok(TtsEvent::EndOfStream)
            }
            other => {
                error!(msg_type = %other, "Unknown TTS message type");
                Err(Error::UnknownMessageType(other.to_string()))
            }
        }
    }

}
