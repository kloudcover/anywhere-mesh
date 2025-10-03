use std::collections::HashMap;
use std::convert::Infallible;

use base64::{engine::general_purpose, Engine as _};
use hyper::{Body, Request, Response, StatusCode};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::service::CombinedIngressService;
use crate::common::{routing, IngressMessage};

#[derive(Clone, Debug)]
pub struct WsInitAck {
    pub success: bool,
    pub message: Option<String>,
    #[allow(dead_code)]
    pub response_headers: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug)]
pub struct WsSession {
    #[allow(dead_code)]
    pub agent_connection_id: Uuid,
    pub alb_out_tx: mpsc::UnboundedSender<AlbOutboundFrame>,
}

#[derive(Clone, Debug)]
pub enum AlbOutboundFrame {
    Text(String),
    Binary(Vec<u8>),
    Close(Option<u16>, Option<String>),
}

impl CombinedIngressService {
    pub fn ws_proxy_enabled() -> bool {
        std::env::var("ENABLE_ALB_WS_PROXY")
            .ok()
            .map(|s| s.to_lowercase() != "false" && s != "0")
            .unwrap_or(true)
    }

    pub async fn start_ws_tunnel_from_alb(
        &self,
        req: Request<Body>,
    ) -> Result<Response<Body>, Infallible> {
        if !Self::ws_proxy_enabled() {
            return Ok(Response::builder()
                .status(StatusCode::NOT_IMPLEMENTED)
                .body(Body::from("WebSocket proxying is disabled"))
                .unwrap());
        }

        let upgrade_is_ws = req
            .headers()
            .get("upgrade")
            .and_then(|h| h.to_str().ok())
            .map(|h| h.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false);

        if !upgrade_is_ws {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("WebSocket upgrade required"))
                .unwrap());
        }

        // Extract values we need before moving the request
        let key = req
            .headers()
            .get("sec-websocket-key")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Extract host for routing and path for the target
        let host = req
            .headers()
            .get("x-forwarded-host")
            .or_else(|| req.headers().get("host"))
            .and_then(|h| h.to_str().ok())
            .map(|s| s.split(':').next().unwrap_or(s).to_string())
            .or_else(|| req.uri().authority().map(|a| a.host().to_string()))
            .unwrap_or_else(|| "unknown".to_string());

        let path = req
            .uri()
            .path_and_query()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "/".to_string());

        // Collect minimal headers to forward
        let mut fwd_headers: HashMap<String, String> = HashMap::new();
        for name in [
            "host",
            "x-forwarded-for",
            "x-forwarded-proto",
            "x-forwarded-host",
            "sec-websocket-protocol",
            "cookie",
            "authorization",
        ] {
            if let Some(value) = req.headers().get(name) {
                if let Ok(value_str) = value.to_str() {
                    fwd_headers.insert(name.to_string(), value_str.to_string());
                }
            }
        }

        // Choose a target agent by host
        let registrations = match self.registry.get_all_registrations().await {
            Ok(map) => map,
            Err(e) => {
                error!("Failed to fetch registrations: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from("No upstream available"))
                    .unwrap());
            }
        };

        let connections = match self.registry.get_all_connections().await {
            Ok(map) => map,
            Err(e) => {
                error!("Failed to fetch connections: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from("No upstream available"))
                    .unwrap());
            }
        };

        let regs: Vec<_> = registrations.values().cloned().collect();
        let maybe_match = routing::match_host_to_service(&host, &regs);
        let Some(matched_service) = maybe_match else {
            warn!("No matching service for host {}", host);
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Service Not Found"))
                .unwrap());
        };

        // Check health
        if routing::select_healthy_instance(std::slice::from_ref(matched_service), &connections)
            .is_none()
        {
            warn!("Matched service unhealthy for host {}", host);
            return Ok(Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(Body::from("No healthy service available"))
                .unwrap());
        }

        // Prepare and send init message to the agent connection
        let session_id = Uuid::new_v4();
        info!(
            "Starting ALB WebSocket tunnel session {} for host {}",
            session_id, host
        );

        // Some clients use multiple subprotocols; pass-through via headers field, but also
        // provide structured subprotocols if present
        let subprotocols = fwd_headers.get("sec-websocket-protocol").map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .collect::<Vec<_>>()
        });

        let agent_sender = if let Some(sender) = self
            .registry
            .get_connection_sender(matched_service.id)
            .await
            .ok()
            .flatten()
        {
            let init = IngressMessage::WebSocketProxyInit {
                session_id,
                target_host: host.clone(),
                path: path.clone(),
                headers: fwd_headers.clone(),
                subprotocols,
            };
            if let Err(e) = sender.send(init) {
                error!("Failed to send WS init to agent: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from("Upstream init failed"))
                    .unwrap());
            }
            sender
        } else {
            warn!("No sender found for matched agent {}", matched_service.id);
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from("No upstream connection"))
                .unwrap());
        };

        // Register session and create a waiter for init ack
        let (to_alb_tx, mut to_alb_rx) = mpsc::unbounded_channel::<AlbOutboundFrame>();
        {
            let mut sessions = self.ws_sessions.write().await;
            sessions.insert(
                session_id,
                WsSession {
                    agent_connection_id: matched_service.id,
                    alb_out_tx: to_alb_tx.clone(),
                },
            );
        }

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<WsInitAck>();
        {
            let mut waiters = self.ws_init_waiters.write().await;
            waiters.insert(session_id, ack_tx);
        }

        let accept_key = derive_accept_key(key.as_bytes());

        let service_for_task = self.clone();
        let agent_sender_for_task = agent_sender.clone();
        tokio::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    let stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
                        upgraded,
                        tokio_tungstenite::tungstenite::protocol::Role::Server,
                        None,
                    )
                    .await;

                    // Wait for init ack result before proceeding
                    match ack_rx.await {
                        Ok(ack) if ack.success => {
                            info!("WS session {} init ack success", session_id);
                            use futures_util::SinkExt as _;
                            use futures_util::StreamExt as _;
                            let (mut alb_sink, mut alb_stream) =
                                futures_util::StreamExt::split(stream);

                            // Task: forward frames from ALB -> agent
                            let agent_sender_in = agent_sender_for_task.clone();
                            let alb_to_agent = tokio::spawn(async move {
                                while let Some(next) = alb_stream.next().await {
                                    match next {
                                        Ok(WsMessage::Text(text)) => {
                                            let _ = agent_sender_in.send(
                                                IngressMessage::WebSocketProxyData {
                                                    session_id,
                                                    frame_type: "text".to_string(),
                                                    payload: Some(text),
                                                },
                                            );
                                        }
                                        Ok(WsMessage::Binary(bytes)) => {
                                            let b64 = general_purpose::STANDARD.encode(bytes);
                                            let _ = agent_sender_in.send(
                                                IngressMessage::WebSocketProxyData {
                                                    session_id,
                                                    frame_type: "binary".to_string(),
                                                    payload: Some(b64),
                                                },
                                            );
                                        }
                                        Ok(WsMessage::Ping(_)) => {
                                            let _ = agent_sender_in.send(
                                                IngressMessage::WebSocketProxyData {
                                                    session_id,
                                                    frame_type: "ping".to_string(),
                                                    payload: None,
                                                },
                                            );
                                        }
                                        Ok(WsMessage::Pong(_)) => {
                                            let _ = agent_sender_in.send(
                                                IngressMessage::WebSocketProxyData {
                                                    session_id,
                                                    frame_type: "pong".to_string(),
                                                    payload: None,
                                                },
                                            );
                                        }
                                        Ok(WsMessage::Close(frame)) => {
                                            let (code, reason) = frame
                                                .map(|f| (None, Some(f.reason)))
                                                .unwrap_or((None, None));
                                            let _ = agent_sender_in.send(
                                                IngressMessage::WebSocketProxyClose {
                                                    session_id,
                                                    code,
                                                    reason: reason.map(|r| r.to_string()),
                                                },
                                            );
                                            break;
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            warn!(
                                                "ALB ws read error for session {}: {}",
                                                session_id, e
                                            );
                                            break;
                                        }
                                    }
                                }
                            });

                            // Task: forward frames from server (via channel) -> ALB
                            let agent_to_alb = tokio::spawn(async move {
                                while let Some(frame) = to_alb_rx.recv().await {
                                    let send_result = match frame {
                                        AlbOutboundFrame::Text(s) => {
                                            alb_sink.send(WsMessage::Text(s)).await
                                        }
                                        AlbOutboundFrame::Binary(b) => {
                                            alb_sink.send(WsMessage::Binary(b)).await
                                        }
                                        AlbOutboundFrame::Close(_code, _reason) => {
                                            alb_sink.send(WsMessage::Close(None)).await
                                        }
                                    };
                                    if let Err(e) = send_result {
                                        warn!(
                                            "ALB ws send error for session {}: {}",
                                            session_id, e
                                        );
                                        break;
                                    }
                                }
                            });

                            let _ = tokio::join!(alb_to_agent, agent_to_alb);
                            // Notify agent that ALB side closed
                            let _ =
                                agent_sender_for_task.send(IngressMessage::WebSocketProxyClose {
                                    session_id,
                                    code: None,
                                    reason: Some("alb connection closed".to_string()),
                                });
                        }
                        Ok(ack) => {
                            warn!("WS session {} init failed: {:?}", session_id, ack.message);
                            // Stream will be dropped on scope end, closing connection.
                        }
                        Err(_) => {
                            warn!("WS session {} init waiter dropped", session_id);
                        }
                    }

                    // Cleanup waiter and session if still present
                    let _ = service_for_task
                        .ws_init_waiters
                        .write()
                        .await
                        .remove(&session_id);
                    let _ = service_for_task
                        .ws_sessions
                        .write()
                        .await
                        .remove(&session_id);
                }
                Err(e) => {
                    error!("Failed to upgrade ALB connection for WS: {}", e);
                }
            }
        });

        Ok(Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header("upgrade", "websocket")
            .header("connection", "upgrade")
            .header("sec-websocket-accept", accept_key)
            .body(Body::empty())
            .unwrap())
    }

    pub async fn handle_ws_proxy_init_ack(
        &self,
        session_id: Uuid,
        success: bool,
        _message: Option<String>,
        _response_headers: Option<HashMap<String, String>>,
    ) {
        let ack = WsInitAck {
            success,
            message: _message,
            response_headers: _response_headers,
        };

        if let Some(tx) = self.ws_init_waiters.write().await.remove(&session_id) {
            let _ = tx.send(ack);
        } else {
            warn!("Init ack waiter not found for session {}", session_id);
        }
    }

    pub async fn handle_ws_proxy_data_from_agent(
        &self,
        session_id: Uuid,
        frame_type: String,
        payload: Option<String>,
    ) {
        let sessions = self.ws_sessions.read().await;
        if let Some(sess) = sessions.get(&session_id) {
            match frame_type.as_str() {
                "text" => {
                    if let Some(text) = payload {
                        let _ = sess.alb_out_tx.send(AlbOutboundFrame::Text(text));
                    }
                }
                "binary" => {
                    if let Some(b64) = payload {
                        if let Ok(bytes) = general_purpose::STANDARD.decode(b64) {
                            let _ = sess.alb_out_tx.send(AlbOutboundFrame::Binary(bytes));
                        }
                    }
                }
                "ping" => {
                    // Many libs auto-handle ping/pong; we can ignore or respond
                }
                "pong" => {}
                _ => {}
            }
        } else {
            warn!("Received WS data for unknown session {}", session_id);
        }
    }

    pub async fn handle_ws_proxy_close_from_agent(
        &self,
        session_id: Uuid,
        _code: Option<u16>,
        _reason: Option<String>,
    ) {
        let mut sessions = self.ws_sessions.write().await;
        if let Some(sess) = sessions.remove(&session_id) {
            let _ = sess
                .alb_out_tx
                .send(AlbOutboundFrame::Close(_code, _reason.clone()));
            info!("WS proxy session {} closed by agent", session_id);
        } else {
            warn!("Unknown session close from agent: {}", session_id);
        }
    }
}
