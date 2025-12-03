//! Message types for the Gradium TTS/STT WebSocket protocol.

use serde::{Deserialize, Serialize};

// ============================================================================
// Common
// ============================================================================

/// Generic message with just a type field, used for initial parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericMessage {
    /// The message type.
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// End of stream message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EosMessage {
    /// The message type (always "end_of_stream").
    #[serde(rename = "type")]
    pub msg_type: String,
}

impl EosMessage {
    /// Creates a new end of stream message.
    pub fn new() -> Self {
        Self {
            msg_type: "end_of_stream".to_string(),
        }
    }
}

impl Default for EosMessage {
    fn default() -> Self {
        Self::new()
    }
}

/// Error message from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMessage {
    /// The message type (always "error").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Error message.
    pub message: String,
    /// Error code.
    pub code: i32,
}

// ============================================================================
// TTS Messages
// ============================================================================

/// TTS setup message sent to initialize the TTS session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsSetupMessage {
    /// The message type (always "setup").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Voice ID to use for synthesis.
    pub voice_id: String,
    /// Model name to use.
    pub model_name: String,
    /// Output audio format.
    pub output_format: String,
}

impl TtsSetupMessage {
    /// Creates a new TTS setup message.
    pub fn new(voice_id: String, model_name: String, output_format: String) -> Self {
        Self {
            msg_type: "setup".to_string(),
            voice_id,
            model_name,
            output_format,
        }
    }
}

/// TTS ready message received when the server is ready.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsReadyMessage {
    /// The message type (always "ready").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Request ID for this session.
    pub request_id: String,
}

/// TTS text message to send text for synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsTextMessage {
    /// The message type (always "text").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Text to synthesize.
    pub text: String,
}

impl TtsTextMessage {
    /// Creates a new TTS text message.
    pub fn new(text: String) -> Self {
        Self {
            msg_type: "text".to_string(),
            text,
        }
    }
}

/// TTS audio message received containing synthesized audio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsAudioMessage {
    /// The message type (always "audio").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Base64-encoded audio data.
    pub audio: String,
}

// ============================================================================
// STT Messages
// ============================================================================

/// STT setup message sent to initialize the STT session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttSetupMessage {
    /// The message type (always "setup").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Model name to use.
    pub model_name: String,
    /// Input audio format.
    pub input_format: String,
}

impl SttSetupMessage {
    /// Creates a new STT setup message.
    pub fn new(model_name: String, input_format: String) -> Self {
        Self {
            msg_type: "setup".to_string(),
            model_name,
            input_format,
        }
    }
}

/// STT ready message received when the server is ready.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttReadyMessage {
    /// The message type (always "ready").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Request ID for this session.
    pub request_id: String,
    /// Model name being used.
    pub model_name: String,
    /// Expected sample rate.
    pub sample_rate: i32,
}

/// STT audio message to send audio for recognition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttAudioMessage {
    /// The message type (always "audio").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Base64-encoded audio data.
    pub audio: String,
}

impl SttAudioMessage {
    /// Creates a new STT audio message.
    pub fn new(audio: String) -> Self {
        Self {
            msg_type: "audio".to_string(),
            audio,
        }
    }
}

/// STT text message received containing recognized text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttTextMessage {
    /// The message type (always "text").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Recognized text.
    pub text: String,
    /// Start time in seconds.
    #[serde(rename = "start_s")]
    pub start: f32,
}

/// VAD (Voice Activity Detection) entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VadEntry {
    /// Horizon in seconds.
    #[serde(rename = "horizon_s")]
    pub horizon: f32,
    /// Inactivity probability.
    pub inactivity_prob: f32,
}

/// STT step message with VAD information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttStepMessage {
    /// The message type (always "step").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Step index.
    pub step_idx: i32,
    /// Step duration in seconds.
    #[serde(rename = "step_duration_s")]
    pub step_duration: f32,
    /// VAD entries.
    pub vad: Vec<VadEntry>,
    /// Total duration in seconds.
    #[serde(rename = "total_duration_s")]
    pub total_duration: f32,
}

/// STT end text message indicating end of a recognized phrase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttEndTextMessage {
    /// The message type (always "end_text").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Stop time in seconds.
    #[serde(rename = "stop_s")]
    pub stop: f32,
}

