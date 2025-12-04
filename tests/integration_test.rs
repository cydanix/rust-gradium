//! Integration tests for the Gradium TTS/STT client library.
//!
//! To run these tests, set the GRADIUM_API_KEY environment variable.

use rust_gradium::{SttClient, SttConfig, SttEvent, TtsClient, TtsConfig, TtsEvent, DEFAULT_VOICE_ID, STT_ENDPOINT, TTS_ENDPOINT};
use tracing::info;

fn get_api_key() -> Option<String> {
    std::env::var("GRADIUM_API_KEY").ok()
}

#[tokio::test]
async fn test_tts_connection() {
    let api_key = match get_api_key() {
        Some(key) => key,
        None => {
            eprintln!("Skipping test: GRADIUM_API_KEY not set");
            return;
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = TtsConfig::new(
        TTS_ENDPOINT.to_string(),
        DEFAULT_VOICE_ID.to_string(),
        api_key,
    );

    let (client, _events) = TtsClient::new(config);

    let result = client.start().await;
    assert!(result.is_ok(), "Failed to start TTS: {:?}", result.err());
    assert!(client.is_ready(), "TTS client should be ready");
    assert!(client.is_running(), "TTS client should be running");

    client.shutdown().await;
}

#[tokio::test]
async fn test_tts_synthesis() {
    use base64::Engine;

    let api_key = match get_api_key() {
        Some(key) => key,
        None => {
            eprintln!("Skipping test: GRADIUM_API_KEY not set");
            return;
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = TtsConfig::new(
        TTS_ENDPOINT.to_string(),
        DEFAULT_VOICE_ID.to_string(),
        api_key,
    );

    let (client, mut events) = TtsClient::new(config);
    client.start().await.expect("Failed to start TTS");

    // Send text for synthesis
    client.process("Hello, world!").await.expect("Failed to send text");

    // Collect audio chunks from events
    let mut audio_chunks = Vec::new();
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(event) = events.recv().await {
            match event {
                TtsEvent::Audio { audio } => {
                    audio_chunks.push(audio);
                }
                TtsEvent::EndOfStream => {
                    break;
                }
                _ => {}
            }
        }
    });

    client.shutdown().await;
    let _ = timeout.await;

    eprintln!("Received {} audio chunks", audio_chunks.len());
    assert!(!audio_chunks.is_empty(), "Should have received audio chunks");

    // Verify audio is base64 encoded
    for chunk in &audio_chunks {
        let decoded = base64::engine::general_purpose::STANDARD.decode(chunk);
        assert!(decoded.is_ok(), "Audio chunk should be valid base64");
    }
}

#[tokio::test]
async fn test_stt_connection() {
    let api_key = match get_api_key() {
        Some(key) => key,
        None => {
            eprintln!("Skipping test: GRADIUM_API_KEY not set");
            return;
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = SttConfig::new(STT_ENDPOINT.to_string(), api_key);

    let (client, _events) = SttClient::new(config);

    let result = client.start().await;
    assert!(result.is_ok(), "Failed to start STT: {:?}", result.err());
    assert!(client.is_ready(), "STT client should be ready");
    assert!(client.is_running(), "STT client should be running");

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_tts_stt_round_trip() {
    use base64::Engine;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let api_key = match get_api_key() {
        Some(key) => key,
        None => {
            eprintln!("Skipping test: GRADIUM_API_KEY not set");
            return;
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    // FIR filter coefficients for downsampling
    const FIR: [f64; 11] = [
        -0.010, -0.020, -0.010,
        0.040, 0.120, 0.180,
        0.120, 0.040, -0.010,
        -0.020, -0.010,
    ];

    // Downsample 48kHz to 24kHz with low-pass filter
    fn downsample_48_to_24(input: &[u8]) -> Vec<u8> {
        // Convert bytes to i16 samples
        let samples48: Vec<i16> = input
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();

        let filter_half = FIR.len() / 2;
        let mut out = Vec::with_capacity(samples48.len() / 2);

        // Apply LPF + decimate (take every 2 samples)
        let mut i = filter_half;
        while i < samples48.len().saturating_sub(filter_half) {
            let mut acc = 0.0f64;
            for k in 0..FIR.len() {
                acc += samples48[i - filter_half + k] as f64 * FIR[k];
            }
            acc = acc.clamp(-32768.0, 32767.0);
            out.push(acc as i16);
            i += 2;
        }

        // Pack back into bytes
        out.iter()
            .flat_map(|&s| s.to_le_bytes())
            .collect()
    }

    fn downsample_48_to_24_base64(input: &str) -> String {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(input) {
            Ok(b) => b,
            Err(_) => return String::new(),
        };
        base64::engine::general_purpose::STANDARD.encode(downsample_48_to_24(&bytes))
    }

    // Start TTS
    let tts_config = TtsConfig::new(
        TTS_ENDPOINT.to_string(),
        DEFAULT_VOICE_ID.to_string(),
        api_key.clone(),
    );
    let (tts, mut tts_events) = TtsClient::new(tts_config);
    tts.start().await.expect("Failed to start TTS");

    // Start STT
    let stt_config = SttConfig::new(STT_ENDPOINT.to_string(), api_key);
    let (stt, mut stt_events) = SttClient::new(stt_config);
    stt.start().await.expect("Failed to start STT");

    let full_text = Arc::new(Mutex::new(String::new()));
    let stt = Arc::new(stt);
    let tts = Arc::new(tts);

    // Spawn audio forwarding task: TTS events -> STT (must run concurrently!)
    let stt_clone = Arc::clone(&stt);
    let audio_task = tokio::spawn(async move {
        while let Some(event) = tts_events.recv().await {
            match event {
                TtsEvent::Audio { audio } => {
                    let downsampled = downsample_48_to_24_base64(&audio);
                    if let Err(e) = stt_clone.process(&downsampled).await {
                        eprintln!("Failed to process audio: {}", e);
                        break;
                    }
                }
                TtsEvent::EndOfStream => {
                    info!("TTS EndOfStream received");
                    break;
                }
                _ => {}
            }
        }
    });

    // Spawn text collection task: STT events -> full_text (must run concurrently!)
    let full_text_clone = Arc::clone(&full_text);
    let text_task = tokio::spawn(async move {
        while let Some(event) = stt_events.recv().await {
            match event {
                SttEvent::Text { text, .. } => {
                    info!("STT text: {}", text);
                    let mut full = full_text_clone.lock().await;
                    full.push(' ');
                    full.push_str(&text);
                }
                SttEvent::EndOfStream => {
                    info!("STT EndOfStream received");
                    break;
                }
                _ => {}
            }
        }
    });

    // Original text to synthesize
    let original_text = r#"Under the soft buzz of a desk lamp and the quiet pulse of background fans, the world sharpens into a single stream of focus, a ribbon of thought that stitches ideas to action without pause; memories of half-finished sketches and dog-eared notebooks dissolve, giving way to the clarity that arrives when curiosity finally outruns hesitation, and you start turning the questions over like stones in a creek, listening for the click of possibility in the current; here, the scratch of keys is steady, the notes stack into little cairns of progress, and every small decision—naming a function, tilting a sentence, nudging a color—whispers its loyalty to the larger shape taking form; there is no lightning, only the warm patience of iteration, the knowing that craft is a long conversation with your future self, who will be grateful you tightened this bolt and trimmed that sentence; beyond the window, a tram shivers along the rails and someone laughs on the street, but the room holds its breath as you test, adjust, and test again, discovering that momentum is less a push than a gentle fall forward, a quiet agreement between intent and attention; in this rhythm you can hear the old promise of making things that last, things that will carry someone else for a few steps when their feet are tired, and you realize the point was never speed, nor cleverness, but the kindness of well-shaped work; so you keep laying down useful lines—of text, of code, of thought—trusting that clarity prefers companions, trusting that time will sand the edges and reveal the grain, trusting that somewhere a reader's morning will be eased, a learner's path unknotted, a teammate's day slightly lighter, because you tended to the details here, now, while the lamp hums, the fans turn, and the quiet room becomes, briefly, a small and generous workshop of the world."#;

    // Send words one by one
    for word in original_text.split_whitespace() {
        tts.process(word).await.expect("Failed to enqueue text");
    }

    // Shutdown TTS (sends EOS, triggers EndOfStream event)
    tts.shutdown().await;

    // Wait for audio task to complete (it will exit on TTS EndOfStream)
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), audio_task).await;

    // Shutdown STT (sends EOS, triggers EndOfStream event)
    stt.shutdown().await;

    // Wait for text task to complete (it will exit on STT EndOfStream)
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), text_task).await;

    // Compare results
    let full_text_result = full_text.lock().await.clone();
    let scores = rust_gradium::textsim::compare(&full_text_result, original_text);

    info!("Original text: {}", original_text);
    info!("Full text: {}", full_text_result);
    info!(
        "Scores: WER={:.3}, CER={:.3}, TokenF1={:.3}, Similarity={:.3}",
        scores.wer, scores.cer, scores.token_f1, scores.similarity
    );

    assert!(
        scores.wer <= 0.1,
        "WER too high: {:.3} (expected <= 0.1)",
        scores.wer
    );
    assert!(
        scores.cer <= 0.1,
        "CER too high: {:.3} (expected <= 0.1)",
        scores.cer
    );
    assert!(
        scores.token_f1 >= 0.9,
        "TokenF1 too low: {:.3} (expected >= 0.9)",
        scores.token_f1
    );
    assert!(
        scores.similarity >= 0.95,
        "Similarity too low: {:.3} (expected >= 0.95)",
        scores.similarity
    );
}
