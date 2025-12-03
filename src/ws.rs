//! WebSocket connection wrapper with ping/pong handling.

use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::time::{interval, timeout};
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::error::Error;

const CONN_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(5);
const PING_TIMEOUT: Duration = Duration::from_secs(30);


/// WebSocket connection wrapper with automatic ping/pong handling.
pub struct WebSocket {
    write_tx: mpsc::Sender<Message>,
    read_rx: Arc<Mutex<mpsc::Receiver<Result<Message, Error>>>>,
    stop_notify: Arc<Notify>,
    _ping_handle: tokio::task::JoinHandle<()>,
    _read_handle: tokio::task::JoinHandle<()>,
    _write_handle: tokio::task::JoinHandle<()>,
}

impl WebSocket {
    /// Opens a new WebSocket connection to the given URL with the provided API key.
    pub async fn connect(url: &str, api_key: &str) -> Result<Self, Error> {
        info!(url = %url, "WebSocket connecting");

        let request = Request::builder()
            .uri(url)
            .header("x-api-key", api_key)
            .header("Host", extract_host(url))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .expect("Failed to build request");

        let (ws_stream, _) = timeout(CONN_TIMEOUT, tokio_tungstenite::connect_async(request))
            .await
            .map_err(|_| Error::ConnectionTimeout)?
            .map_err(Error::WebSocket)?;

        info!(url = %url, "WebSocket connected");

        let (write, read) = ws_stream.split();
        let write = Arc::new(Mutex::new(write));
        let read = Arc::new(Mutex::new(read));

        // Channel for outgoing messages
        let (write_tx, mut write_rx) = mpsc::channel::<Message>(256);

        // Channel for incoming messages (to be consumed by the client)
        let (read_tx, read_rx) = mpsc::channel::<Result<Message, Error>>(256);
        let read_rx = Arc::new(Mutex::new(read_rx));

        let stop_notify = Arc::new(Notify::new());

        // Write task
        let write_clone = Arc::clone(&write);
        let stop_notify_clone = Arc::clone(&stop_notify);
        let write_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop_notify_clone.notified() => {
                        debug!("Write task stopping");
                        break;
                    }
                    msg = write_rx.recv() => {
                        match msg {
                            Some(msg) => {
                                let msg_desc = match &msg {
                                    Message::Text(t) => format!("Text({})", t.len()),
                                    Message::Binary(b) => format!("Binary({})", b.len()),
                                    Message::Ping(_) => "Ping".to_string(),
                                    Message::Pong(_) => "Pong".to_string(),
                                    Message::Close(_) => "Close".to_string(),
                                    Message::Frame(_) => "Frame".to_string(),
                                };
                                let mut writer = write_clone.lock().await;
                                match writer.send(msg).await {
                                    Ok(_) => {
                                        debug!(msg = %msg_desc, "WebSocket message sent");
                                    }
                                    Err(e) => {
                                        error!(error = %e, "Write error");
                                        break;
                                    }
                                }
                            }
                            None => {
                                debug!("Write channel closed");
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Read task
        let read_clone = Arc::clone(&read);
        let stop_notify_clone = Arc::clone(&stop_notify);
        let write_tx_clone = write_tx.clone();
        let read_handle = tokio::spawn(async move {
            loop {
                let msg = {
                    let mut reader = read_clone.lock().await;
                    tokio::select! {
                        _ = stop_notify_clone.notified() => {
                            debug!("Read task stopping");
                            break;
                        }
                        msg = reader.next() => msg
                    }
                };

                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        debug!("Received ping, sending pong");
                        let _ = write_tx_clone.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        debug!("Received pong");
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!("Received close");
                        break;
                    }
                    Some(Ok(msg)) => {
                        if read_tx.send(Ok(msg)).await.is_err() {
                            debug!("Read channel closed");
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        error!(error = %e, "Read error");
                        let _ = read_tx.send(Err(Error::WebSocket(e))).await;
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended");
                        break;
                    }
                }
            }
        });

        // Ping task
        let write_tx_clone = write_tx.clone();
        let stop_notify_clone = Arc::clone(&stop_notify);
        let ping_handle = tokio::spawn(async move {
            let mut ping_interval = interval(PING_INTERVAL);
            ping_interval.tick().await; // Skip immediate tick

            loop {
                tokio::select! {
                    _ = stop_notify_clone.notified() => {
                        debug!("Ping task stopping");
                        break;
                    }
                    _ = ping_interval.tick() => {
                        debug!("Sending ping");
                        if write_tx_clone.send(Message::Ping(b"ping".to_vec())).await.is_err() {
                            warn!("Failed to send ping");
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            write_tx,
            read_rx,
            stop_notify,
            _ping_handle: ping_handle,
            _read_handle: read_handle,
            _write_handle: write_handle,
        })
    }

    /// Sends a text message.
    pub async fn send_text(&self, text: &str) -> Result<(), Error> {
        self.write_tx
            .send(Message::Text(text.to_string()))
            .await
            .map_err(|_| Error::ChannelSend)
    }

    /// Sends a binary message.
    #[allow(dead_code)]
    pub async fn send_binary(&self, data: Vec<u8>) -> Result<(), Error> {
        self.write_tx
            .send(Message::Binary(data))
            .await
            .map_err(|_| Error::ChannelSend)
    }

    /// Receives the next message with a timeout.
    pub async fn recv(&self) -> Result<Message, Error> {
        let mut rx = self.read_rx.lock().await;
        timeout(PING_TIMEOUT, rx.recv())
            .await
            .map_err(|_| Error::ConnectionTimeout)?
            .ok_or(Error::ChannelRecv)?
    }

    /// Closes the WebSocket connection.
    pub async fn close(&self) -> Result<(), Error> {
        info!("WebSocket closing");
        self.stop_notify.notify_waiters();
        let _ = self
            .write_tx
            .send(Message::Close(None))
            .await;
        info!("WebSocket closed");
        Ok(())
    }
}

fn extract_host(url: &str) -> &str {
    url.strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .and_then(|s| s.split('/').next())
        .unwrap_or("localhost")
}

