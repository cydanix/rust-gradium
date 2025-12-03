//! Speech-to-Text client for the Gradium API.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::error::Error;
use crate::messages::*;
use crate::queue::BoundedQueue;
use crate::ws::WebSocket;

const READY_TIMEOUT: Duration = Duration::from_secs(30);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const QUEUE_CAPACITY: usize = 1024;

/// Configuration for the STT client.
#[derive(Debug, Clone)]
pub struct SttConfig {
    /// WebSocket endpoint URL.
    pub endpoint: String,
    /// API key for authentication.
    pub api_key: String,
    /// Model name (default: "default").
    pub model_name: String,
    /// Input format (default: "pcm").
    pub input_format: String,
}

impl SttConfig {
    /// Creates a new STT configuration with default model and format.
    pub fn new(endpoint: String, api_key: String) -> Self {
        Self {
            endpoint,
            api_key,
            model_name: "default".to_string(),
            input_format: "pcm".to_string(),
        }
    }
}

/// Speech-to-Text client for streaming audio recognition.
pub struct SttClient {
    config: SttConfig,
    conn: RwLock<Option<Arc<WebSocket>>>,
    text_queue: Arc<BoundedQueue<String>>,
    ready: Arc<AtomicBool>,
    error_count: Arc<AtomicU32>,
    stopping: Arc<AtomicBool>,
    read_task: RwLock<Option<tokio::task::JoinHandle<()>>>,
    session_id: String,
}

impl SttClient {
    /// Creates a new STT client with the given configuration.
    pub fn new(config: SttConfig) -> Self {
        Self {
            config,
            conn: RwLock::new(None),
            text_queue: Arc::new(BoundedQueue::new(QUEUE_CAPACITY)),
            ready: Arc::new(AtomicBool::new(false)),
            error_count: Arc::new(AtomicU32::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            read_task: RwLock::new(None),
            session_id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
        }
    }

    /// Starts the STT client, connecting to the server and waiting for ready state.
    pub async fn start(&self) -> Result<(), Error> {
        info!(session_id = %self.session_id, "STT starting");

        // Reset state
        self.ready.store(false, Ordering::SeqCst);
        self.error_count.store(0, Ordering::SeqCst);
        self.stopping.store(false, Ordering::SeqCst);
        self.text_queue.clear().await;

        // Connect
        let conn = WebSocket::connect(&self.config.endpoint, &self.config.api_key).await?;
        let conn = Arc::new(conn);
        *self.conn.write().await = Some(Arc::clone(&conn));

        // Send setup
        self.send_setup().await?;

        // Start read task
        let read_conn = Arc::clone(&conn);
        let text_queue = Arc::clone(&self.text_queue);
        let ready = Arc::clone(&self.ready);
        let error_count = Arc::clone(&self.error_count);
        let session_id = self.session_id.clone();

        *self.read_task.write().await = Some(tokio::spawn(async move {
            Self::read_messages(read_conn, text_queue, ready, error_count, session_id).await;
        }));

        // Wait for ready
        if !self.wait_ready().await {
            self.shutdown().await;
            return Err(Error::ReadyTimeout);
        }

        info!(session_id = %self.session_id, "STT started");
        Ok(())
    }

    /// Shuts down the STT client.
    ///
    /// Sends end_of_stream, waits for all text to be received, then closes the connection.
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);

        info!(session_id = %self.session_id, "STT shutting down");

        // Send EOS - this triggers the server to send remaining text and then end_of_stream
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

        info!(session_id = %self.session_id, "STT shut down");
    }

    /// Sends audio for recognition.
    ///
    /// The audio should be base64-encoded PCM data.
    /// Returns an error if the client is not ready or is stopping.
    pub async fn process(&self, audio: &str) -> Result<(), Error> {
        if !self.is_running() {
            return Err(Error::NotReady);
        }
        self.send_audio(audio).await
    }

    /// Retrieves up to `size` text chunks from the queue.
    pub async fn get_text(&self, size: usize) -> Vec<String> {
        self.text_queue.get(size).await
    }

    /// Returns the number of text chunks available in the queue.
    pub async fn get_text_size(&self) -> usize {
        self.text_queue.len().await
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
            debug!("Waiting for STT ready");
            if self.error_count.load(Ordering::SeqCst) > 0 {
                break;
            }
            sleep(READY_POLL_INTERVAL).await;
        }
        let is_ready =
            self.ready.load(Ordering::SeqCst) && self.error_count.load(Ordering::SeqCst) == 0;
        debug!(ready = is_ready, "STT ready check complete");
        is_ready
    }

    async fn send_setup(&self) -> Result<(), Error> {
        info!("Sending STT setup");
        let payload = SttSetupMessage::new(
            self.config.model_name.clone(),
            self.config.input_format.clone(),
        );
        let json = serde_json::to_string(&payload)?;
        self.conn.read().await.as_ref().unwrap().send_text(&json).await
    }

    async fn send_audio(&self, audio: &str) -> Result<(), Error> {
        debug!("Sending STT audio");
        let payload = SttAudioMessage::new(audio.to_string());
        let json = serde_json::to_string(&payload)?;
        self.conn.read().await.as_ref().unwrap().send_text(&json).await
    }

    async fn send_eos(&self) -> Result<(), Error> {
        debug!("Sending STT EOS");
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
        text_queue: Arc<BoundedQueue<String>>,
        ready: Arc<AtomicBool>,
        error_count: Arc<AtomicU32>,
        session_id: String,
    ) {
        info!(session_id = %session_id, "STT reading messages");

        loop {
            let msg = match conn.recv().await {
                Ok(msg) => msg,
                Err(e) => {
                    error!(error = %e, "STT read error");
                    return;
                }
            };

            let text = match msg {
                Message::Text(t) => t,
                Message::Binary(b) => match String::from_utf8(b) {
                    Ok(s) => s,
                    Err(e) => {
                        error!(error = %e, "Invalid UTF-8 in binary message");
                        continue;
                    }
                },
                Message::Close(_) => return,
                _ => continue,
            };

            let generic: GenericMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(e) => {
                    error!(error = %e, "Failed to parse message");
                    return;
                }
            };

            match generic.msg_type.as_str() {
                "ready" => {
                    let msg: SttReadyMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse ready message");
                            return;
                        }
                    };
                    info!(
                        request_id = %msg.request_id,
                        model_name = %msg.model_name,
                        sample_rate = msg.sample_rate,
                        "STT ready"
                    );
                    ready.store(true, Ordering::SeqCst);
                }
                "text" => {
                    let msg: SttTextMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse text message");
                            return;
                        }
                    };
                    debug!(text = %msg.text, start = msg.start, "STT word");
                    // Add space prefix like the Go implementation
                    if let Err(e) = text_queue.append(vec![format!(" {}", msg.text)]).await {
                        error!(error = %e, "Text queue append error");
                        return;
                    }
                }
                "end_text" => {
                    let msg: SttEndTextMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse end_text message");
                            return;
                        }
                    };
                    debug!(stop = msg.stop, "STT end text");
                }
                "step" => {
                    // Step messages with VAD info - just log at debug level
                    let _msg: SttStepMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse step message");
                            return;
                        }
                    };
                    // Don't log step messages by default as they're verbose
                }
                "error" => {
                    let msg: ErrorMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            error!(error = %e, "Failed to parse error message");
                            return;
                        }
                    };
                    error!(message = %msg.message, code = msg.code, "STT error");
                    error_count.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                "end_of_stream" => {
                    info!("STT end of stream");
                    return;
                }
                other => {
                    error!(msg_type = %other, "Unknown STT message type");
                    return;
                }
            }
        }
    }
}

