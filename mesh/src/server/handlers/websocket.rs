use hyper::{Body, Method, Request, Response, StatusCode};
use std::convert::Infallible;
use tracing::{error, instrument};

use crate::server::CombinedIngressService;

impl CombinedIngressService {
    #[instrument(skip(self, req))]
    pub async fn handle_websocket_or_health_request(
        &self,
        req: Request<Body>,
    ) -> Result<Response<Body>, Infallible> {
        match (req.method(), req.uri().path()) {
            (&Method::GET, "/health") => {
                // Handle health check on WebSocket port
                let connections_count = self
                    .registry
                    .get_all_connections()
                    .await
                    .map(|c| c.len())
                    .unwrap_or(0);
                let registrations_count = self
                    .registry
                    .get_all_registrations()
                    .await
                    .map(|r| r.len())
                    .unwrap_or(0);
                let instance_id = self.server_instance_id;
                let started_at = self
                    .started_at
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_else(|_| std::time::Duration::from_secs(0))
                    .as_secs();

                // TODO: add metrics
                let health_info = format!(
                    "{{\"status\":\"healthy\",\"connections\":{},\"registrations\":{},\"port\":\"8082\",\"instance_id\":\"{}\",\"started_at\":{}}}",
                    connections_count, registrations_count, instance_id, started_at
                );

                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(health_info))
                    .unwrap())
            }
            (&Method::GET, _) => {
                // Check if this is a WebSocket upgrade request
                if req
                    .headers()
                    .get("upgrade")
                    .and_then(|h| h.to_str().ok())
                    .map(|h| h.to_lowercase() == "websocket")
                    .unwrap_or(false)
                {
                    // Extract WebSocket key before moving the request
                    let key = req
                        .headers()
                        .get("sec-websocket-key")
                        .and_then(|h| h.to_str().ok())
                        .unwrap_or("")
                        .to_string();

                    let accept_key = tokio_tungstenite::tungstenite::handshake::derive_accept_key(
                        key.as_bytes(),
                    );

                    // Handle WebSocket upgrade
                    let service = self.clone();
                    tokio::spawn(async move {
                        match hyper::upgrade::on(req).await {
                            Ok(upgraded) => {
                                let stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
                                    upgraded,
                                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                                    None,
                                )
                                .await;

                                if let Err(e) = service.handle_websocket_stream(stream).await {
                                    error!("WebSocket connection error: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Failed to upgrade connection: {}", e);
                            }
                        }
                    });

                    // Return WebSocket handshake response
                    Ok(Response::builder()
                        .status(StatusCode::SWITCHING_PROTOCOLS)
                        .header("upgrade", "websocket")
                        .header("connection", "upgrade")
                        .header("sec-websocket-accept", accept_key)
                        .body(Body::empty())
                        .unwrap())
                } else {
                    // Regular HTTP request, not a WebSocket upgrade
                    Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Body::from("WebSocket upgrade required"))
                        .unwrap())
                }
            }
            _ => {
                // Unsupported method
                Ok(Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .body(Body::from("Method not allowed"))
                    .unwrap())
            }
        }
    }
}
