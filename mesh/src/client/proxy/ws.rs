use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose, Engine as _};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::common::IngressMessage;

#[derive(Clone, Debug)]
enum OutboundToLocal {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Pong,
    Close(Option<u16>, Option<String>),
}

#[derive(Clone)]
pub struct WebSocketReverseProxy {
    pub local_endpoint: String,
    sessions: Arc<RwLock<HashMap<Uuid, mpsc::UnboundedSender<OutboundToLocal>>>>,
}

impl WebSocketReverseProxy {
    pub fn new(local_endpoint: String) -> Self {
        Self {
            local_endpoint,
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn to_ws_url(&self, path: &str) -> String {
        let mut base = self.local_endpoint.clone();
        if base.starts_with("https://") {
            base = base.replacen("https://", "wss://", 1);
        } else if base.starts_with("http://") {
            base = base.replacen("http://", "ws://", 1);
        }
        format!("{}{}", base.trim_end_matches('/'), path)
    }

    pub async fn handle_init(
        &self,
        session_id: Uuid,
        _target_host: String,
        path: String,
        _headers: HashMap<String, String>,
        _subprotocols: Option<Vec<String>>,
        upstream_tx: &mpsc::UnboundedSender<IngressMessage>,
    ) {
        let ws_url = self.to_ws_url(&path);
        info!(
            "WS reverse proxy init for session {} to {}",
            session_id, ws_url
        );

        match connect_async(&ws_url).await {
            Ok((ws_stream, _resp)) => {
                let (mut local_sink, mut local_stream) = ws_stream.split();

                let (to_local_tx, mut to_local_rx) = mpsc::unbounded_channel::<OutboundToLocal>();
                {
                    let mut sessions = self.sessions.write().await;
                    sessions.insert(session_id, to_local_tx.clone());
                }

                let writer = tokio::spawn(async move {
                    while let Some(frame) = to_local_rx.recv().await {
                        let res = match frame {
                            OutboundToLocal::Text(s) => local_sink.send(WsMessage::Text(s)).await,
                            OutboundToLocal::Binary(b) => {
                                local_sink.send(WsMessage::Binary(b)).await
                            }
                            OutboundToLocal::Ping => local_sink.send(WsMessage::Ping(vec![])).await,
                            OutboundToLocal::Pong => local_sink.send(WsMessage::Pong(vec![])).await,
                            OutboundToLocal::Close(_code, _reason) => {
                                local_sink.send(WsMessage::Close(None)).await
                            }
                        };
                        if let Err(e) = res {
                            warn!("Local ws send error for session {}: {}", session_id, e);
                            break;
                        }
                    }
                });

                let upstream_tx2 = upstream_tx.clone();
                let reader = tokio::spawn(async move {
                    while let Some(next) = local_stream.next().await {
                        match next {
                            Ok(WsMessage::Text(text)) => {
                                let _ = upstream_tx2.send(IngressMessage::WebSocketProxyData {
                                    session_id,
                                    frame_type: "text".to_string(),
                                    payload: Some(text),
                                });
                            }
                            Ok(WsMessage::Binary(bytes)) => {
                                let b64 = general_purpose::STANDARD.encode(bytes);
                                let _ = upstream_tx2.send(IngressMessage::WebSocketProxyData {
                                    session_id,
                                    frame_type: "binary".to_string(),
                                    payload: Some(b64),
                                });
                            }
                            Ok(WsMessage::Ping(_)) => {
                                let _ = upstream_tx2.send(IngressMessage::WebSocketProxyData {
                                    session_id,
                                    frame_type: "ping".to_string(),
                                    payload: None,
                                });
                            }
                            Ok(WsMessage::Pong(_)) => {
                                let _ = upstream_tx2.send(IngressMessage::WebSocketProxyData {
                                    session_id,
                                    frame_type: "pong".to_string(),
                                    payload: None,
                                });
                            }
                            Ok(WsMessage::Close(frame)) => {
                                let (code, reason) = frame
                                    .map(|f| (None, Some(f.reason)))
                                    .unwrap_or((None, None));
                                let _ = upstream_tx2.send(IngressMessage::WebSocketProxyClose {
                                    session_id,
                                    code,
                                    reason: reason.map(|r| r.to_string()),
                                });
                                break;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                warn!("Local ws read error for session {}: {}", session_id, e);
                                break;
                            }
                        }
                    }
                });

                let _ = upstream_tx.send(IngressMessage::WebSocketProxyInitAck {
                    session_id,
                    success: true,
                    message: None,
                    response_headers: None,
                });

                let cleanup_sessions = self.sessions.clone();
                tokio::spawn(async move {
                    let _ = tokio::join!(writer, reader);
                    let mut sessions = cleanup_sessions.write().await;
                    sessions.remove(&session_id);
                });
            }
            Err(e) => {
                error!(
                    "Failed to connect to local ws for session {}: {}",
                    session_id, e
                );
                let _ = upstream_tx.send(IngressMessage::WebSocketProxyInitAck {
                    session_id,
                    success: false,
                    message: Some(format!("{}", e)),
                    response_headers: None,
                });
            }
        }
    }

    pub async fn handle_data_from_server(
        &self,
        session_id: Uuid,
        frame_type: String,
        payload: Option<String>,
    ) {
        let sessions = self.sessions.read().await;
        if let Some(tx) = sessions.get(&session_id) {
            match frame_type.as_str() {
                "text" => {
                    if let Some(text) = payload {
                        let _ = tx.send(OutboundToLocal::Text(text));
                    }
                }
                "binary" => {
                    if let Some(b64) = payload {
                        if let Ok(bytes) = general_purpose::STANDARD.decode(b64) {
                            let _ = tx.send(OutboundToLocal::Binary(bytes));
                        }
                    }
                }
                "ping" => {
                    let _ = tx.send(OutboundToLocal::Ping);
                }
                "pong" => {
                    let _ = tx.send(OutboundToLocal::Pong);
                }
                _ => {}
            }
        } else {
            warn!("Data for unknown ws session {} (client side)", session_id);
        }
    }

    pub async fn handle_close_from_server(
        &self,
        session_id: Uuid,
        code: Option<u16>,
        reason: Option<String>,
    ) {
        let mut sessions = self.sessions.write().await;
        if let Some(tx) = sessions.remove(&session_id) {
            let _ = tx.send(OutboundToLocal::Close(code, reason));
        } else {
            info!("Close for unknown ws session {} (client side)", session_id);
        }
    }
}
