# rust-gradium

Rust client library for [Gradium AI](https://gradium.ai) Text-to-Speech (TTS) and Speech-to-Text (STT) WebSocket APIs.

## Features

- **Text-to-Speech (TTS)**: Stream text to synthesized audio
- **Speech-to-Text (STT)**: Stream audio for real-time transcription
- **Event-driven API**: Pull events via `next_event()` async method
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

    let client = TtsClient::new(config);
    client.start().await?;

    // Send text for synthesis
    client.process("Hello, world!").await?;
    client.send_eos().await?;

    // Receive audio chunks via next_event()
    loop {
        match client.next_event().await? {
            TtsEvent::Audio { audio } => {
                // audio is base64-encoded PCM
                println!("Received audio chunk: {} bytes", audio.len());
            }
            TtsEvent::EndOfStream | TtsEvent::Close => break,
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

    let client = SttClient::new(config);
    client.start().await?;

    // Send audio for recognition (base64-encoded PCM)
    let audio_base64 = "..."; // Your base64-encoded audio data
    client.process(audio_base64).await?;
    client.send_eos().await?;

    // Receive text via next_event()
    let mut full_text = String::new();
    loop {
        match client.next_event().await? {
            SttEvent::Text { text, .. } => {
                full_text.push_str(&text);
                println!("Recognized: {}", text);
            }
            SttEvent::Step { vad, .. } => {
                // VAD entries contain inactivity probabilities at different time horizons
                for entry in vad {
                    if entry.inactivity_prob > 0.5 {
                        println!("User likely inactive (prob: {:.2} at {}s horizon)", 
                            entry.inactivity_prob, entry.horizon);
                    }
                }
            }
            SttEvent::EndOfStream | SttEvent::Close => break,
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
    downsample_48_to_24_base64,
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), rust_gradium::Error> {
    let api_key = std::env::var("GRADIUM_API_KEY").expect("GRADIUM_API_KEY not set");

    // Initialize TTS
    let tts = TtsClient::new(TtsConfig::new(
        TTS_ENDPOINT.to_string(),
        DEFAULT_VOICE_ID.to_string(),
        api_key.clone(),
    ));
    tts.start().await?;

    // Initialize STT
    let stt = SttClient::new(SttConfig::new(
        STT_ENDPOINT.to_string(),
        api_key,
    ));
    stt.start().await?;

    let tts = Arc::new(tts);
    let stt = Arc::new(stt);
    let full_text = Arc::new(Mutex::new(String::new()));

    // Task: Forward TTS audio to STT
    let tts_clone = Arc::clone(&tts);
    let stt_clone = Arc::clone(&stt);
    let audio_task = tokio::spawn(async move {
        loop {
            match tts_clone.next_event().await {
                Ok(TtsEvent::Audio { audio }) => {
                    // Downsample 48kHz -> 24kHz for STT
                    let downsampled = downsample_48_to_24_base64(&audio);
                    let _ = stt_clone.process(&downsampled).await;
                }
                Ok(TtsEvent::EndOfStream) | Ok(TtsEvent::Close) => break,
                Err(_) => break,
                _ => {}
            }
        }
        tts_clone.shutdown().await;
    });

    // Task: Collect STT text
    let stt_clone2 = Arc::clone(&stt);
    let full_text_clone = Arc::clone(&full_text);
    let text_task = tokio::spawn(async move {
        loop {
            match stt_clone2.next_event().await {
                Ok(SttEvent::Text { text, .. }) => {
                    full_text_clone.lock().await.push_str(&text);
                }
                Ok(SttEvent::EndOfStream) | Ok(SttEvent::Close) => break,
                Err(_) => break,
                _ => {}
            }
        }
        stt_clone2.shutdown().await;
    });

    // Generate speech
    tts.process("Hello, how are you?").await?;
    tts.send_eos().await?;
    let _ = audio_task.await;

    // Signal STT end of stream
    stt.send_eos().await?;
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
| `new(config) -> Self` | Create a new TTS client |
| `start().await` | Connect and initialize the session |
| `process(text).await` | Send text for synthesis |
| `send_eos().await` | Signal end of input stream |
| `next_event().await` | Receive the next event |
| `is_ready()` | Check if client is ready |
| `is_running()` | Check if client is ready and running |
| `error_count()` | Get the current error count |
| `shutdown().await` | Close the connection |

### TtsEvent

| Variant | Description |
|---------|-------------|
| `Ready { request_id }` | Client is ready |
| `Audio { audio }` | Base64-encoded PCM audio chunk (48kHz) |
| `TextEcho { text }` | Echo of sent text |
| `Error { message, code }` | Error occurred |
| `EndOfStream` | Stream ended |
| `Close` | WebSocket connection closed |
| `Ping` | Ping received |
| `Pong` | Pong received |

### SttClient

| Method | Description |
|--------|-------------|
| `new(config) -> Self` | Create a new STT client |
| `start().await` | Connect and initialize the session |
| `process(audio).await` | Send base64-encoded audio for recognition (24kHz PCM) |
| `send_eos().await` | Signal end of input stream |
| `next_event().await` | Receive the next event |
| `is_ready()` | Check if client is ready |
| `is_running()` | Check if client is ready and running |
| `error_count()` | Get the current error count |
| `shutdown().await` | Close the connection |

### SttEvent

| Variant | Description |
|---------|-------------|
| `Ready { request_id, model_name, sample_rate }` | Client is ready |
| `Text { text, start }` | Recognized text with timestamp |
| `EndText { stop }` | End of phrase |
| `Step { step_idx, step_duration, vad, total_duration }` | Processing step with VAD entries |
| `Error { message, code }` | Error occurred |
| `EndOfStream` | Stream ended |
| `Close` | WebSocket connection closed |
| `Ping` | Ping received |
| `Pong` | Pong received |

### VadEntry

| Field | Description |
|-------|-------------|
| `horizon` | Time horizon in seconds |
| `inactivity_prob` | Probability user is inactive (0.0 - 1.0) |

### Utility Functions

| Function | Description |
|----------|-------------|
| `downsample_48_to_24_base64(input)` | Downsample base64 audio from 48kHz to 24kHz |

## Testing

Set the `GRADIUM_API_KEY` environment variable and run:

```bash
GRADIUM_API_KEY=your-key cargo test
```

## License

MIT
