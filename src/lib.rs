//! Rust client library for Gradium AI Text-to-Speech (TTS) and Speech-to-Text (STT) WebSocket APIs.
//!
//! # Example
//!
//! ```no_run
//! use rust_gradium::{TtsClient, TtsConfig, TtsEvent};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rust_gradium::Error> {
//!     let config = TtsConfig {
//!         endpoint: rust_gradium::TTS_ENDPOINT.to_string(),
//!         voice_id: rust_gradium::DEFAULT_VOICE_ID.to_string(),
//!         api_key: std::env::var("GRADIUM_API_KEY").expect("GRADIUM_API_KEY not set"),
//!         model_name: "default".to_string(),
//!         output_format: "pcm".to_string(),
//!     };
//!
//!     let client = TtsClient::new(config);
//!     client.start().await?;
//!
//!     client.process("Hello, world!").await?;
//!     client.send_eos().await?;
//!
//!     // Receive audio chunks via next_event
//!     loop {
//!         match client.next_event().await? {
//!             TtsEvent::Audio { audio } => {
//!                 // Process base64-encoded audio
//!                 println!("Received audio chunk: {} bytes", audio.len());
//!             }
//!             TtsEvent::EndOfStream => break,
//!             _ => {}
//!         }
//!     }
//!
//!     client.shutdown().await;
//!     Ok(())
//! }
//! ```

mod error;
mod messages;
mod stt;
pub mod textsim;
mod tts;
pub mod wg;
mod ws;
mod downsample;

pub use downsample::{downsample_48_to_24, downsample_48_to_24_base64};
pub use error::Error;
pub use wg::{WaitGroup, WaitGroupGuard};
pub use messages::*;
pub use stt::{SttClient, SttConfig, SttEvent};
pub use tts::{TtsClient, TtsConfig, TtsEvent};

/// Default TTS WebSocket endpoint.
pub const TTS_ENDPOINT: &str = "wss://us.api.gradium.ai/api/speech/tts";

/// Default STT WebSocket endpoint.
pub const STT_ENDPOINT: &str = "wss://us.api.gradium.ai/api/speech/asr";

/// Default voice ID for TTS.
pub const DEFAULT_VOICE_ID: &str = "LFZvm12tW_z0xfGo";

