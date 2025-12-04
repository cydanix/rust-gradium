# rust-gradium

Rust client library for [Gradium AI](https://gradium.ai) Text-to-Speech (TTS) and Speech-to-Text (STT) WebSocket APIs.

## Features

- **Text-to-Speech (TTS)**: Stream text to synthesized audio
- **Speech-to-Text (STT)**: Stream audio for real-time transcription
- **Event-driven API**: Receive audio/text via async event channels
- Async/await with Tokio runtime
- Automatic WebSocket ping/pong handling

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
rust-gradium = "0.1"
tokio = { version = "1.0", features = ["rt-multi-thread", "macros"] }
```

## Quick Start

### Text-to-Speech

```rust
use rust_gradium::{TtsClient, TtsConfig, TtsEvent, TTS_ENDPOINT, DEFAULT_VOICE_ID};

#[tokio::main]
async fn main() -> Result<(), rust_gradium::Error> {
    let config = TtsConfig::new(
        TTS_ENDPOINT.to_string(),
        DEFAULT_VOICE_ID.to_string(),
        std::env::var("GRADIUM_API_KEY").expect("GRADIUM_API_KEY not set"),
    );

    let (client, mut events) = TtsClient::new(config);
    client.start().await?;

    // Send text for synthesis
    client.process("Hello, world!").await?;

    // Receive audio chunks via events
    while let Some(event) = events.recv().await {
        match event {
            TtsEvent::Audio { audio } => {
                // audio is base64-encoded PCM
                println!("Received audio chunk: {} bytes", audio.len());
            }
            TtsEvent::EndOfStream => break,
            _ => {}
        }
    }

    client.shutdown().await;
    Ok(())
}
```

### Speech-to-Text

```rust
use rust_gradium::{SttClient, SttConfig, SttEvent, STT_ENDPOINT};

#[tokio::main]
async fn main() -> Result<(), rust_gradium::Error> {
    let config = SttConfig::new(
        STT_ENDPOINT.to_string(),
        std::env::var("GRADIUM_API_KEY").expect("GRADIUM_API_KEY not set"),
    );

    let (client, mut events) = SttClient::new(config);
    client.start().await?;

    // Send audio for recognition (base64-encoded PCM)
    let audio_base64 = "..."; // Your base64-encoded audio data
    client.process(audio_base64).await?;

    // Receive text via events
    let mut full_text = String::new();
    while let Some(event) = events.recv().await {
        match event {
            SttEvent::Text { text, .. } => {
                full_text.push_str(&text);
                println!("Recognized: {}", text);
            }
            SttEvent::UserInactive { inactivity_prob } => {
                println!("User inactive (prob: {:.2})", inactivity_prob);
            }
            SttEvent::EndOfStream => break,
            _ => {}
        }
    }

    client.shutdown().await;
    Ok(())
}
```

### TTS + STT Round Trip

```rust
use rust_gradium::{
    TtsClient, TtsConfig, TtsEvent,
    SttClient, SttConfig, SttEvent,
    TTS_ENDPOINT, STT_ENDPOINT, DEFAULT_VOICE_ID,
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), rust_gradium::Error> {
    let api_key = std::env::var("GRADIUM_API_KEY").expect("GRADIUM_API_KEY not set");

    // Initialize TTS
    let (tts, mut tts_events) = TtsClient::new(TtsConfig::new(
        TTS_ENDPOINT.to_string(),
        DEFAULT_VOICE_ID.to_string(),
        api_key.clone(),
    ));
    tts.start().await?;

    // Initialize STT
    let (stt, mut stt_events) = SttClient::new(SttConfig::new(
        STT_ENDPOINT.to_string(),
        api_key,
    ));
    stt.start().await?;

    let stt = Arc::new(stt);
    let full_text = Arc::new(Mutex::new(String::new()));

    // Task: Forward TTS audio to STT
    let stt_clone = Arc::clone(&stt);
    let audio_task = tokio::spawn(async move {
        while let Some(event) = tts_events.recv().await {
            match event {
                TtsEvent::Audio { audio } => {
                    // In production, downsample 48kHz -> 24kHz if needed
                    let _ = stt_clone.process(&audio).await;
                }
                TtsEvent::EndOfStream => break,
                _ => {}
            }
        }
    });

    // Task: Collect STT text
    let full_text_clone = Arc::clone(&full_text);
    let text_task = tokio::spawn(async move {
        while let Some(event) = stt_events.recv().await {
            match event {
                SttEvent::Text { text, .. } => {
                    full_text_clone.lock().await.push_str(&text);
                }
                SttEvent::EndOfStream => break,
                _ => {}
            }
        }
    });

    // Generate speech
    tts.process("Hello, how are you?").await?;

    // Shutdown (triggers EndOfStream events)
    tts.shutdown().await;
    let _ = audio_task.await;

    stt.shutdown().await;
    let _ = text_task.await;

    println!("Round-trip result: {}", full_text.lock().await);
    Ok(())
}
```

## Configuration

### TTS Configuration

```rust
use rust_gradium::TtsConfig;

let config = TtsConfig {
    endpoint: "wss://us.api.gradium.ai/api/speech/tts".to_string(),
    voice_id: "LFZvm12tW_z0xfGo".to_string(),
    api_key: "your-api-key".to_string(),
    model_name: "default".to_string(),    // optional
    output_format: "pcm".to_string(),     // optional
};
```

### STT Configuration

```rust
use rust_gradium::SttConfig;

let config = SttConfig {
    endpoint: "wss://us.api.gradium.ai/api/speech/asr".to_string(),
    api_key: "your-api-key".to_string(),
    model_name: "default".to_string(),    // optional
    input_format: "pcm".to_string(),      // optional
};
```

## API Reference

### TtsClient

| Method | Description |
|--------|-------------|
| `new(config) -> (Self, Receiver)` | Create a new TTS client and event receiver |
| `start().await` | Connect and initialize the session |
| `process(text).await` | Send text for synthesis |
| `is_running()` | Check if client is ready and running |
| `shutdown().await` | Close the connection |

### TtsEvent

| Variant | Description |
|---------|-------------|
| `Ready { request_id }` | Client is ready |
| `Audio { audio }` | Base64-encoded PCM audio chunk |
| `TextEcho { text }` | Echo of sent text |
| `Error { message, code }` | Error occurred |
| `EndOfStream` | Stream ended |

### SttClient

| Method | Description |
|--------|-------------|
| `new(config) -> (Self, Receiver)` | Create a new STT client and event receiver |
| `start().await` | Connect and initialize the session |
| `process(audio).await` | Send base64-encoded audio for recognition |
| `is_running()` | Check if client is ready and running |
| `shutdown().await` | Close the connection |

### SttEvent

| Variant | Description |
|---------|-------------|
| `Ready { request_id, model_name, sample_rate }` | Client is ready |
| `Text { text, start }` | Recognized text with timestamp |
| `EndText { stop }` | End of phrase |
| `UserInactive { inactivity_prob }` | User is inactive (prob > 0.5) |
| `Error { message, code }` | Error occurred |
| `EndOfStream` | Stream ended |

## Testing

Set the `GRADIUM_API_KEY` environment variable and run:

```bash
GRADIUM_API_KEY=your-key cargo test
```

## License

MIT
