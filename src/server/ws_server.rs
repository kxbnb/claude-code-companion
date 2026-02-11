use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::app::AppEvent;
use crate::protocol::types::CliMessage;

pub struct WsServer {
    port: u16,
    event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl WsServer {
    pub fn new(port: u16, event_tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        Self { port, event_tx }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port)).await?;
        tracing::info!("WebSocket server listening on 127.0.0.1:{}", self.port);

        loop {
            let (stream, addr) = listener.accept().await?;
            tracing::debug!("TCP connection from {}", addr);
            let event_tx = self.event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, event_tx).await {
                    tracing::error!("WebSocket connection error: {}", e);
                }
            });
        }
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    // Extract session ID from the URL path during WebSocket handshake
    let session_id_cell = Arc::new(Mutex::new(None::<String>));
    let sid_clone = session_id_cell.clone();

    let callback =
        move |req: &http::Request<()>,
              resp: http::Response<()>|
              -> Result<http::Response<()>, http::Response<Option<String>>> {
            let path = req.uri().path();
            tracing::debug!("WebSocket upgrade request for path: {}", path);

            if let Some(id) = path.strip_prefix("/ws/cli/") {
                let id = id.trim_end_matches('/');
                if !id.is_empty() {
                    *sid_clone.lock().unwrap() = Some(id.to_string());
                    return Ok(resp);
                }
            }

            // Reject connections that don't match the expected path
            let err_resp = http::Response::builder()
                .status(http::StatusCode::NOT_FOUND)
                .body(Some("Expected path: /ws/cli/{session_id}".to_string()))
                .unwrap();
            Err(err_resp)
        };

    let ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
    let session_id = session_id_cell
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| anyhow::anyhow!("No session ID extracted from WebSocket path"))?;

    tracing::info!("CLI WebSocket connected for session {}", session_id);

    // Create a channel for sending messages back to this CLI connection
    let (cli_tx, mut cli_rx) = mpsc::unbounded_channel::<String>();

    // Notify the event loop that the CLI connected
    let _ = event_tx.send(AppEvent::CliConnected {
        session_id: session_id.clone(),
        sender: cli_tx,
    });

    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Read task: WebSocket → event loop
    let sid_read = session_id.clone();
    let etx_read = event_tx.clone();
    let read_handle = tokio::spawn(async move {
        while let Some(msg_result) = ws_read.next().await {
            match msg_result {
                Ok(Message::Text(text)) => {
                    let text_str = text.to_string();
                    for line in text_str.split('\n').filter(|l| !l.trim().is_empty()) {
                        match serde_json::from_str::<CliMessage>(line) {
                            Ok(cli_msg) => {
                                let _ = etx_read.send(AppEvent::CliMessage {
                                    session_id: sid_read.clone(),
                                    message: cli_msg,
                                });
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to parse CLI NDJSON: {} — {}",
                                    e,
                                    &line[..line.len().min(200)]
                                );
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("CLI WebSocket closed for session {}", sid_read);
                    break;
                }
                Err(e) => {
                    tracing::error!("WebSocket read error for session {}: {}", sid_read, e);
                    break;
                }
                _ => {} // Ping/Pong handled automatically
            }
        }
    });

    // Write task: event loop → WebSocket
    let write_handle = tokio::spawn(async move {
        while let Some(ndjson) = cli_rx.recv().await {
            // NDJSON requires newline delimiter
            let payload = if ndjson.ends_with('\n') {
                ndjson
            } else {
                format!("{}\n", ndjson)
            };
            if ws_write.send(Message::text(payload)).await.is_err() {
                break;
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = read_handle => {}
        _ = write_handle => {}
    }

    tracing::info!("CLI WebSocket session {} ended", session_id);
    let _ = event_tx.send(AppEvent::CliDisconnected { session_id });

    Ok(())
}
