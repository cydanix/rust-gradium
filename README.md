# rust-gradium

Rust client library for [Gradium AI](https://gradium.ai) Text-to-Speech (TTS) and Speech-to-Text (STT) WebSocket APIs.

## Features

- **Text-to-Speech (TTS)**: Stream text to synthesized audio
- **Speech-to-Text (STT)**: Stream audio for real-time transcription
- Async/await with Tokio runtime
- Automatic WebSocket ping/pong handling
- Thread-safe bounded queues for audio/text buffering

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
use rust_gradium::{TtsClient, TtsConfig, TTS_ENDPOINT, DEFAULT_VOICE_ID};

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

    // Wait for audio to be generated
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Get audio chunks (base64-encoded PCM)
    let audio_chunks = client.get_speech(100).await;
    println!("Received {} audio chunks", audio_chunks.len());

    client.shutdown().await;
    Ok(())
}
```

### Speech-to-Text

```rust
use rust_gradium::{SttClient, SttConfig, STT_ENDPOINT};

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

    // Wait for transcription
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Get recognized text
    let text_chunks = client.get_text(100).await;
    let full_text: String = text_chunks.join("");
    println!("Recognized: {}", full_text);

    client.shutdown().await;
    Ok(())
}
```

### TTS + STT Round Trip

```rust
use rust_gradium::{
    TtsClient, TtsConfig, SttClient, SttConfig,
    TTS_ENDPOINT, STT_ENDPOINT, DEFAULT_VOICE_ID,
};

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

    // Generate speech
    tts.process("Hello, how are you?").await?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Send generated audio to STT
    for audio_chunk in tts.get_speech(100).await {
        stt.process(&audio_chunk).await?;
    }

    // Get transcription
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let text: String = stt.get_text(100).await.join("");
    println!("Round-trip result: {}", text);

    tts.shutdown().await;
    stt.shutdown().await;
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
| `new(config)` | Create a new TTS client |
| `start().await` | Connect and initialize the session |
| `process(text).await` | Send text for synthesis |
| `get_speech(size).await` | Get up to `size` audio chunks |
| `get_speech_size().await` | Get number of available audio chunks |
| `is_running()` | Check if client is ready and running |
| `shutdown().await` | Close the connection |

### SttClient

| Method | Description |
|--------|-------------|
| `new(config)` | Create a new STT client |
| `start().await` | Connect and initialize the session |
| `process(audio).await` | Send base64-encoded audio for recognition |
| `get_text(size).await` | Get up to `size` text chunks |
| `get_text_size().await` | Get number of available text chunks |
| `is_running()` | Check if client is ready and running |
| `shutdown().await` | Close the connection |

## Testing

Set the `GRADIUM_API_KEY` environment variable and run:

```bash
GRADIUM_API_KEY=your-key cargo test
```

## License

MIT

