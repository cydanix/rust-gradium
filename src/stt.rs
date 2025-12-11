//! Speech-to-Text client for the Gradium API.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info};
use base64::Engine;

use crate::error::Error;
use crate::messages::*;
use crate::ws::WebSocket;


/// Events emitted by the STT client.
#[derive(Debug, Clone)]
pub enum SttEvent {
    /// The STT client is ready.
    Ready {
        /// Request ID for this session.
        request_id: String,
        /// Model name being used.
        model_name: String,
        /// Expected sample rate.
        sample_rate: usize,

        frame_size: usize,
        delay_in_frames: usize,
    },
    /// A step of the STT process.
    Step {
        /// Step index.
        step_idx: i32,
        /// Step duration in seconds.
        step_duration: f32,
        /// VAD entries.
        vad: Vec<VadEntry>,
        /// Total duration in seconds.
        total_duration: f32,
    },
    /// A word/text was recognized.
    Text {
        /// Recognized text.
        text: String,
        /// Start time in seconds.
        start: f32,
    },
    /// End of a recognized phrase.
    EndText {
        /// Stop time in seconds.
        stop: f32,
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
    /// Language to use for recognition.
    /// Defaults to "en".
    pub language: String,
}

impl SttConfig {
    /// Creates a new STT configuration with default model and format.
    pub fn new(endpoint: String, api_key: String, language: String) -> Self {
        Self {
            endpoint,
            api_key,
            model_name: "default".to_string(),
            input_format: "pcm".to_string(),
            language: language,
        }
    }
}

pub struct SttSettings {
    sample_rate: usize,
    frame_size: usize,
    delay_in_frames: usize,
    silence_str: String,
}

/// Speech-to-Text client for streaming audio recognition.
pub struct SttClient {
    config: SttConfig,
    conn: RwLock<Option<Arc<WebSocket>>>,
    ready: Arc<AtomicBool>,
    error_count: Arc<AtomicU32>,
    stopping: Arc<AtomicBool>,
    session_id: String,

    settings: RwLock<SttSettings>,
}

impl SttClient {
    /// Creates a new STT client with the given configuration.
    ///
    /// Returns the client and an event receiver for receiving STT events.
    pub fn new(config: SttConfig) -> Self {
        let client = Self {
            config,
            conn: RwLock::new(None),
            ready: Arc::new(AtomicBool::new(false)),
            error_count: Arc::new(AtomicU32::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            session_id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            settings: RwLock::new(SttSettings {
                sample_rate: 0,
                frame_size: 0,
                delay_in_frames: 0,
                silence_str: "".to_string(),
            }),
        };
        client
    }

    /// Starts the STT client, connecting to the server and waiting for ready state.
    pub async fn start(&self) -> Result<(), Error> {
        info!(session_id = %self.session_id, "STT starting");

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
                SttEvent::Ready { request_id, model_name, sample_rate, frame_size, delay_in_frames} => {
                    info!(request_id = %request_id,
                        model_name = %model_name,
                        sample_rate = sample_rate,
                        frame_size = frame_size,
                        delay_in_frames = delay_in_frames,
                        "STT ready");
                    *self.settings.write().await = SttSettings {
                        sample_rate: sample_rate as usize,
                        frame_size: frame_size as usize,
                        delay_in_frames: delay_in_frames as usize,
                        silence_str: base64::engine::general_purpose::STANDARD.encode(vec![0; frame_size * delay_in_frames]),
                    };
                    self.ready.store(true, Ordering::SeqCst);
                    break;
                }
                SttEvent::Error { message, code } => {
                    error!(message = %message, code = code, "STT error");
                    self.error_count.fetch_add(1, Ordering::SeqCst);
                    return Err(Error::ServerError { message, code });
                }
                _ => {
                    return Err(Error::UnexpectedEventType);
                }
            }
        }


        info!(session_id = %self.session_id, "STT started");
        Ok(())
    }

    /// Shuts down the STT client.
    ///
    /// Sends end_of_stream, waits for all text to be received, then closes the connection.
    pub async fn shutdown(&self) {
        info!(session_id = %self.session_id, "STT shutting down");

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
        info!("Sending STT setup");
        let payload = SttSetupMessage::new(
            self.config.model_name.clone(),
            self.config.input_format.clone(),
            self.config.language.clone(),
        );
        let json = serde_json::to_string(&payload)?;
        self.conn.read().await.as_ref().unwrap().send_text(&json).await
    }

    async fn send_audio(&self, audio: &str) -> Result<(), Error> {
        debug!("Sending STT audio length: {}", audio.len());
        let payload = SttAudioMessage::new(audio.to_string());
        let json = serde_json::to_string(&payload)?;
        self.conn.read().await.as_ref().unwrap().send_text(&json).await
    }

    pub async fn send_silence(&self) -> Result<(), Error> {
        let settings = self.settings.read().await;
        self.send_audio(&settings.silence_str).await?;
        Ok(())
    }

    pub async fn send_eos(&self) -> Result<(), Error> {
        debug!("Sending STT EOS");
        self.stopping.store(true, Ordering::SeqCst);

        let payload = EosMessage::new();
        let json = serde_json::to_string(&payload)?;
        if let Some(conn) = self.conn.read().await.as_ref() {
            conn.send_text(&json).await
        } else {
            Ok(())
        }
    }

    /// Sends a ping message to keep the connection alive.
    pub async fn send_ping(&self) -> Result<(), Error> {
        if let Some(conn) = self.conn.read().await.as_ref() {
            conn.send_ping().await
        } else {
            Ok(())
        }
    }

    pub async fn next_event(&self) -> Result<SttEvent, Error> {
        let msg = match self.conn.read().await.as_ref().unwrap().recv().await {
            Ok(msg) => msg,
            Err(e) => return Err(e),
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Binary(b) => match String::from_utf8(b) {
                Ok(s) => s,
                Err(e) => {
                    error!(error = %e, "Invalid UTF-8 in binary message");
                    return Err(Error::InvalidUtf8);
                }
            },
            Message::Ping(data) => {
                debug!("STT received ping, sending pong");
                if let Some(conn) = self.conn.read().await.as_ref() {
                    let _ = conn.send_pong(data.clone()).await;
                }
                return Ok(SttEvent::Ping);
            }
            Message::Pong(_) => {
                debug!("STT received pong");
                return Ok(SttEvent::Pong);
            }
            Message::Close(frame) => {
                debug!(frame = ?frame, "STT received close");
                return Ok(SttEvent::Close);
            }
            Message::Frame(_) => {
                debug!("STT received raw frame");
                return Ok(SttEvent::Frame);
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
            "ready" => {
                let msg: SttReadyMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, text = %text, "Failed to parse ready message");
                        return Err(Error::InvalidJson);
                    }
                };
                info!(
                    request_id = %msg.request_id,
                    model_name = %msg.model_name,
                    sample_rate = msg.sample_rate,
                    "STT ready"
                );
                self.ready.store(true, Ordering::SeqCst);
                Ok(SttEvent::Ready {
                    request_id: msg.request_id,
                    model_name: msg.model_name,
                    sample_rate: msg.sample_rate,
                    frame_size: msg.frame_size,
                    delay_in_frames: msg.delay_in_frames,
                })
            }
            "text" => {
                let msg: SttTextMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, "Failed to parse text message");
                        return Err(Error::InvalidJson);
                    }
                };
                debug!(text = %msg.text, start = msg.start, "STT word");
                Ok(SttEvent::Text {
                    text: msg.text,
                    start: msg.start,
                })
            }
            "end_text" => {
                let msg: SttEndTextMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, "Failed to parse end_text message");
                        return Err(Error::InvalidJson);
                    }
                };
                debug!(stop = msg.stop, "STT end text");
                Ok(SttEvent::EndText { stop: msg.stop })
            }
            "step" => {
                // Step messages with VAD info
                let msg: SttStepMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, "Failed to parse step message");
                        return Err(Error::InvalidJson);
                    }
                };

                Ok(SttEvent::Step {
                    step_idx: msg.step_idx,
                    step_duration: msg.step_duration,
                    vad: msg.vad,
                    total_duration: msg.total_duration,
                })
            }
            "error" => {
                let msg: ErrorMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(error = %e, "Failed to parse error message");
                        return Err(Error::InvalidJson);
                    }
                };
                error!(message = %msg.message, code = msg.code, "STT error");
                self.error_count.fetch_add(1, Ordering::SeqCst);
                Ok(SttEvent::Error {
                    message: msg.message,
                    code: msg.code,
                })
            }
            "end_of_stream" => {
                info!("STT end of stream");
                Ok(SttEvent::EndOfStream)
            }
            other => {
                error!(msg_type = %other, "Unknown STT message type");
                return Err(Error::UnknownMessageType(other.to_string()));
            }
        }

    }

}

