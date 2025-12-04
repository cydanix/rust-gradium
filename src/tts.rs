//! Text-to-Speech client for the Gradium API.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::error::Error;
use crate::messages::*;
use crate::ws::WebSocket;

const READY_TIMEOUT: Duration = Duration::from_secs(30);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
    read_task: RwLock<Option<tokio::task::JoinHandle<()>>>,
    session_id: String,
    event_tx: UnboundedSender<TtsEvent>,
}

impl TtsClient {
    /// Creates a new TTS client with the given configuration.
    ///
    /// Returns the client and an event receiver for receiving TTS events.
    pub fn new(config: TtsConfig) -> (Self, UnboundedReceiver<TtsEvent>) {
        let (event_tx, event_rx) = unbounded_channel();
        let client = Self {
            config,
            conn: RwLock::new(None),
            ready: Arc::new(AtomicBool::new(false)),
            error_count: Arc::new(AtomicU32::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            read_task: RwLock::new(None),
            session_id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            event_tx,
        };
        (client, event_rx)
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

        // Start read task
        let read_conn = Arc::clone(&conn);
        let ready = Arc::clone(&self.ready);
        let error_count = Arc::clone(&self.error_count);
        let session_id = self.session_id.clone();
        let event_tx = self.event_tx.clone();

        *self.read_task.write().await = Some(tokio::spawn(async move {
            Self::read_messages(read_conn, ready, error_count, session_id, event_tx).await;
        }));

        // Wait for ready
        if !self.wait_ready().await {
            self.shutdown().await;
            return Err(Error::ReadyTimeout);
        }

        info!(session_id = %self.session_id, "TTS started");
        Ok(())
    }

    /// Shuts down the TTS client.
    ///
    /// Sends end_of_stream, waits for all audio to be received, then closes the connection.
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);

        info!(session_id = %self.session_id, "TTS shutting down");

        // Send EOS - this triggers the server to send remaining audio and then end_of_stream
        if let Err(e) = self.send_eos().await {
            warn!(error = %e, "Failed to send EOS");
        }

        // Wait for read task to complete (will return when server sends end_of_stream)
        if let Some(task) = self.read_task.write().await.take() {
            let _ = task.await;
        }

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

    async fn wait_ready(&self) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < READY_TIMEOUT && !self.ready.load(Ordering::SeqCst) {
            debug!("Waiting for TTS ready");
            if self.error_count.load(Ordering::SeqCst) > 0 {
                break;
            }
            sleep(READY_POLL_INTERVAL).await;
        }
        let is_ready =
            self.ready.load(Ordering::SeqCst) && self.error_count.load(Ordering::SeqCst) == 0;
        debug!(ready = is_ready, "TTS ready check complete");
        is_ready
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

    async fn send_eos(&self) -> Result<(), Error> {
        debug!("Sending TTS EOS");
        let payload = EosMessage::new();
        let json = serde_json::to_string(&payload)?;
        if let Some(conn) = self.conn.read().await.as_ref() {
            conn.send_text(&json).await
        } else {
            Ok(())
        }
    }

    async fn read_messages(
        conn: Arc<WebSocket>,
        ready: Arc<AtomicBool>,
        error_count: Arc<AtomicU32>,
        session_id: String,
        event_tx: UnboundedSender<TtsEvent>,
    ) {
        info!(session_id = %session_id, "TTS reading messages");

        loop {
            let msg = match conn.recv().await {
                Ok(msg) => msg,
                Err(e) => {
                    error!(error = %e, "TTS read error");
                    return;
                }
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
                            continue;
                        }
                    }
                }
                Message::Ping(_) => {
                    debug!("TTS received ping");
                    continue;
                }
                Message::Pong(_) => {
                    debug!("TTS received pong");
                    continue;
                }
                Message::Close(frame) => {
                    debug!(frame = ?frame, "TTS received close");
                    return;
                }
                Message::Frame(_) => {
                    debug!("TTS received raw frame");
                    continue;
                }
            };

            let generic: GenericMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(e) => {
                    error!(error = %e, "Failed to parse message");
                    return;
                }
            };

            match generic.msg_type.as_str() {
                "audio" => {
                    let msg: TtsAudioMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse audio message");
                            return;
                        }
                    };
                    debug!(audio_len = msg.audio.len(), "TTS audio chunk received");
                    let _ = event_tx.send(TtsEvent::Audio { audio: msg.audio });
                }
                "ready" => {
                    let msg: TtsReadyMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse ready message");
                            return;
                        }
                    };
                    info!(request_id = %msg.request_id, "TTS ready");
                    ready.store(true, Ordering::SeqCst);
                    let _ = event_tx.send(TtsEvent::Ready {
                        request_id: msg.request_id,
                    });
                }
                "text" => {
                    let msg: TtsTextMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse text message");
                            return;
                        }
                    };
                    debug!(text = %msg.text, "TTS text echo");
                    let _ = event_tx.send(TtsEvent::TextEcho { text: msg.text });
                }
                "error" => {
                    let msg: ErrorMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse error message");
                            return;
                        }
                    };
                    error!(message = %msg.message, code = msg.code, "TTS error");
                    error_count.fetch_add(1, Ordering::SeqCst);
                    let _ = event_tx.send(TtsEvent::Error {
                        message: msg.message,
                        code: msg.code,
                    });
                    return;
                }
                "end_of_stream" => {
                    info!("TTS end of stream");
                    let _ = event_tx.send(TtsEvent::EndOfStream);
                    return;
                }
                other => {
                    error!(msg_type = %other, "Unknown TTS message type");
                    return;
                }
            }
        }
    }
}
