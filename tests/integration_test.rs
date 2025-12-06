//! Integration tests for the Gradium TTS/STT client library.
//!
//! To run these tests, set the GRADIUM_API_KEY environment variable.

use rust_gradium::{downsample_48_to_24_base64, SttClient, SttConfig, SttEvent, TtsClient, TtsConfig, TtsEvent, DEFAULT_VOICE_ID, STT_ENDPOINT, TTS_ENDPOINT};
use rust_gradium::wg::WaitGroup;
use tracing::{info, error};

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

    let client = TtsClient::new(config);

    let result = client.start().await;
    assert!(result.is_ok(), "Failed to start TTS: {:?}", result.err());
    assert!(client.is_ready(), "TTS client should be ready");
    assert!(client.is_running(), "TTS client should be running");

    client.send_eos().await.expect("Failed to send EOS");
    loop {
        let event = client.next_event().await.expect("Failed to get next event");
        match event {
            TtsEvent::EndOfStream => {
                break;
            }
            TtsEvent::Close => {
                break;
            },
            TtsEvent::Error { message, code: _ } => {
                error!("TTS error: {}", message);
                break;
            },
            _ => {}
        }
    }
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

    let client = TtsClient::new(config);
    client.start().await.expect("Failed to start TTS");

    // Send text for synthesis
    client.process("Hello, world!").await.expect("Failed to send text");

    client.send_eos().await.expect("Failed to send EOS");
    let mut audio_chunks = Vec::new();
    loop {
        let event = client.next_event().await.expect("Failed to get next event");
        match event {
            TtsEvent::Audio { audio } => {
                audio_chunks.push(audio);
            }
            TtsEvent::EndOfStream => {
                break;
            }
            TtsEvent::Close => {
                break;
            },
            TtsEvent::Error { message, code: _ } => {
                error!("TTS error: {}", message);
                break;
            },
            _ => {}
        }
    }
    client.shutdown().await;

    info!("Received {} audio chunks", audio_chunks.len());
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

    let client = SttClient::new(config);

    let result = client.start().await;
    assert!(result.is_ok(), "Failed to start STT: {:?}", result.err());
    assert!(client.is_ready(), "STT client should be ready");
    assert!(client.is_running(), "STT client should be running");

    client.send_eos().await.expect("Failed to send EOS");
    loop {
        let event = client.next_event().await.expect("Failed to get next event");
        match event {
            SttEvent::EndOfStream => {
                break;
            }
            SttEvent::Close => {
                break;
            },
            SttEvent::Error { message, code: _ } => {
                error!("STT error: {}", message);
                break;
            },
            _ => {}
        }
    }
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_tts_stt_round_trip() {
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

    // Start TTS
    let tts_config = TtsConfig::new(
        TTS_ENDPOINT.to_string(),
        DEFAULT_VOICE_ID.to_string(),
        api_key.clone(),
    );
    let tts = TtsClient::new(tts_config);
    tts.start().await.expect("Failed to start TTS");

    // Start STT
    let stt_config = SttConfig::new(STT_ENDPOINT.to_string(), api_key);
    let stt = SttClient::new(stt_config);
    stt.start().await.expect("Failed to start STT");

    let full_text = Arc::new(Mutex::new(String::new()));
    let stt = Arc::new(stt);
    let tts = Arc::new(tts);

    let wg = WaitGroup::new();

    // Spawn TTS ping task
    let tts_ping = Arc::clone(&tts);
    let wg_guard_tts_ping = wg.add();
    let tts_ping_task = tokio::spawn(async move {
        let _wg_guard = wg_guard_tts_ping;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.tick().await; // Skip immediate tick
        loop {
            interval.tick().await;
            if let Err(e) = tts_ping.send_ping().await {
                info!("TTS ping stopped: {}", e);
                break;
            }
        }
    });

    // Spawn STT ping task
    let stt_ping = Arc::clone(&stt);
    let wg_guard_stt_ping = wg.add();
    let stt_ping_task = tokio::spawn(async move {
        let _wg_guard = wg_guard_stt_ping;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.tick().await; // Skip immediate tick
        loop {
            interval.tick().await;
            if let Err(e) = stt_ping.send_ping().await {
                info!("STT ping stopped: {}", e);
                break;
            }
        }
    });

    // Spawn audio forwarding task: TTS events -> STT (must run concurrently!)
    let stt_clone = Arc::clone(&stt);
    let tts_clone = Arc::clone(&tts);
    let wg_guard_audio = wg.add();
    let audio_task = tokio::spawn(async move {
        let _wg_guard = wg_guard_audio;
        loop {
            let event = match tts_clone.next_event().await {
                Ok(e) => e,
                Err(e) => {
                    error!("TTS next_event error: {}", e);
                    break;
                }
            };
            match event {
                TtsEvent::Audio { audio } => {
                    let downsampled = downsample_48_to_24_base64(&audio);
                    if let Err(e) = stt_clone.process(&downsampled).await {
                        error!("Failed to process audio: {}", e);
                        break;
                    }
                }
                TtsEvent::EndOfStream => {
                    info!("TTS EndOfStream/Close received");
                    break;
                }
                TtsEvent::Close => {
                    info!("TTS Close received");
                    break;
                }
                TtsEvent::Error { message, code: _ } => {
                    error!("TTS error: {}", message);
                    break;
                }
                _ => {}
            }
        }
        tts_clone.shutdown().await;
    });

    // Spawn text collection task: STT events -> full_text (must run concurrently!)
    let full_text_clone = Arc::clone(&full_text);
    let stt_clone2 = Arc::clone(&stt);
    let wg_guard_text = wg.add();
    let text_task = tokio::spawn(async move {
        let _wg_guard = wg_guard_text;
        loop {
            let event = match stt_clone2.next_event().await {
                Ok(e) => e,
                Err(e) => {
                    error!("STT next_event error: {}", e);
                    break;
                }
            };
            match event {
                SttEvent::Text { text, .. } => {
                    //info!("STT text: {}", text);
                    let mut full = full_text_clone.lock().await;
                    full.push(' ');
                    full.push_str(&text);
                }
                SttEvent::EndOfStream => {
                    info!("STT EndOfStream received");
                    break;
                }
                SttEvent::Close => {
                    info!("STT Close received");
                    break;
                }
                SttEvent::Error { message, code: _ } => {
                    error!("STT error: {}", message);
                    break;
                }
                _ => {}
            }
        }
        stt_clone2.shutdown().await;
    });

    // Original text to synthesize
    let original_text = r#"Under the soft buzz of a desk lamp and the quiet pulse of background fans, the world sharpens into a single stream of focus, a ribbon of thought that stitches ideas to action without pause; memories of half-finished sketches and dog-eared notebooks dissolve, giving way to the clarity that arrives when curiosity finally outruns hesitation, and you start turning the questions over like stones in a creek, listening for the click of possibility in the current; here, the scratch of keys is steady, the notes stack into little cairns of progress, and every small decision—naming a function, tilting a sentence, nudging a color—whispers its loyalty to the larger shape taking form; there is no lightning, only the warm patience of iteration, the knowing that craft is a long conversation with your future self, who will be grateful you tightened this bolt and trimmed that sentence; beyond the window, a tram shivers along the rails and someone laughs on the street, but the room holds its breath as you test, adjust, and test again, discovering that momentum is less a push than a gentle fall forward, a quiet agreement between intent and attention; in this rhythm you can hear the old promise of making things that last, things that will carry someone else for a few steps when their feet are tired, and you realize the point was never speed, nor cleverness, but the kindness of well-shaped work; so you keep laying down useful lines—of text, of code, of thought—trusting that clarity prefers companions, trusting that time will sand the edges and reveal the grain, trusting that somewhere a reader's morning will be eased, a learner's path unknotted, a teammate's day slightly lighter, because you tended to the details here, now, while the lamp hums, the fans turn, and the quiet room becomes, briefly, a small and generous workshop of the world."#;

    // Send words one by one
    for word in original_text.split_whitespace() {
        tts.process(word).await.expect("Failed to enqueue text");
    }
    tts.send_eos().await.expect("Failed to send EOS");
    let _ = audio_task.await;

    // Shutdown STT (sends EOS, triggers EndOfStream event)
    stt.send_eos().await.expect("Failed to send EOS");
    let _ = text_task.await;

    // Stop ping tasks
    tts_ping_task.abort();
    stt_ping_task.abort();

    wg.wait().await;

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
