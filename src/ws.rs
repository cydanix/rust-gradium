//! WebSocket connection wrapper.

use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info};

use crate::error::Error;

const CONN_TIMEOUT: Duration = Duration::from_secs(10);
const RECV_TIMEOUT: Duration = Duration::from_secs(30);

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// WebSocket connection wrapper.
pub struct WebSocket {
    write: Arc<Mutex<futures_util::stream::SplitSink<WsStream, Message>>>,
    read: Arc<Mutex<futures_util::stream::SplitStream<WsStream>>>,
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

        Ok(Self {
            write: Arc::new(Mutex::new(write)),
            read: Arc::new(Mutex::new(read)),
        })
    }

    /// Sends a text message.
    pub async fn send_text(&self, text: &str) -> Result<(), Error> {
        let mut writer = self.write.lock().await;
        writer
            .send(Message::Text(text.to_string()))
            .await
            .map_err(Error::WebSocket)
    }

    /// Sends a binary message.
    #[allow(dead_code)]
    pub async fn send_binary(&self, data: Vec<u8>) -> Result<(), Error> {
        let mut writer = self.write.lock().await;
        writer
            .send(Message::Binary(data))
            .await
            .map_err(Error::WebSocket)
    }

    /// Sends a ping message.
    pub async fn send_ping(&self) -> Result<(), Error> {
        debug!("Sending ping");
        let mut writer = self.write.lock().await;
        writer
            .send(Message::Ping(b"ping".to_vec()))
            .await
            .map_err(Error::WebSocket)
    }

    /// Sends a pong message.
    pub async fn send_pong(&self, data: Vec<u8>) -> Result<(), Error> {
        debug!("Sending pong");
        let mut writer = self.write.lock().await;
        writer
            .send(Message::Pong(data))
            .await
            .map_err(Error::WebSocket)
    }

    /// Receives the next message with a timeout.
    pub async fn recv(&self) -> Result<Message, Error> {
        let mut reader = self.read.lock().await;
        match timeout(RECV_TIMEOUT, reader.next()).await {
            Ok(Some(Ok(msg))) => Ok(msg),
            Ok(Some(Err(e))) => Err(Error::WebSocket(e)),
            Ok(None) => Err(Error::ChannelRecv),
            Err(_) => Err(Error::ConnectionTimeout),
        }
    }

    /// Closes the WebSocket connection.
    pub async fn close(&self) -> Result<(), Error> {
        info!("WebSocket closing");
        let mut writer = self.write.lock().await;
        let _ = writer.send(Message::Close(None)).await;
        let _ = writer.close().await;
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
