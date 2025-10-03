use super::auth::{AuthService, DefaultAuthService};
use super::dispatcher::{DefaultMessageDispatcher, MessageDispatcher};
use super::registry::{DefaultRegistry, Registry};
use super::router::{DefaultRouter, Router};
use crate::common::{ProxyRequest, ProxyResponse};
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};

use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, instrument};
use uuid::Uuid;

use hyper::Client as HttpClient;
use hyper_tls::HttpsConnector;

#[derive(Clone)]
pub struct CombinedIngressService {
    pub server_instance_id: Uuid,
    pub started_at: SystemTime,

    // Auth service for IAM authentication
    pub auth_service: Arc<dyn AuthService>,
    // Registry for managing connections and services
    pub registry: Arc<dyn Registry>,
    // Router for request routing and response handling
    pub router: Arc<dyn Router>,
    // Dispatcher for message handling
    pub dispatcher: Arc<dyn MessageDispatcher>,
    pub ws_sessions:
        Arc<tokio::sync::RwLock<std::collections::HashMap<Uuid, super::ws_proxy::WsSession>>>,
    pub ws_init_waiters: Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<
                Uuid,
                tokio::sync::oneshot::Sender<super::ws_proxy::WsInitAck>,
            >,
        >,
    >,
}

impl CombinedIngressService {
    pub fn new() -> Self {
        let https = HttpsConnector::new();
        let http_client = HttpClient::builder().build::<_, hyper::Body>(https);

        // Load configuration for auth service
        let skip_iam_validation = std::env::var("SKIP_IAM_VALIDATION")
            .ok()
            .map(|s| s.to_lowercase() == "true")
            .unwrap_or(false);

        let allowed_role_patterns = std::env::var("ALLOWED_ROLE_ARNS")
            .ok()
            .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
            .unwrap_or_else(|| vec!["*".to_string()]);

        let auth_service = Arc::new(DefaultAuthService::new(
            http_client.clone(),
            allowed_role_patterns,
            skip_iam_validation,
        ));

        let registry = Arc::new(DefaultRegistry::new());
        let router = Arc::new(DefaultRouter::new(Duration::from_secs(30))); // Default 30 second timeout
        let dispatcher = Arc::new(DefaultMessageDispatcher::new());

        Self {
            server_instance_id: Uuid::new_v4(),
            started_at: SystemTime::now(),
            auth_service,
            registry,
            router,
            dispatcher,
            ws_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            ws_init_waiters: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    #[instrument(skip(self, proxy_request))]
    pub async fn route_request_through_websocket(
        &self,
        proxy_request: ProxyRequest,
    ) -> Result<ProxyResponse> {
        match self
            .router
            .route_request(proxy_request, self.registry.as_ref())
            .await
        {
            Ok(response) => Ok(response),
            Err(e) => Err(e.into()),
        }
    }

    #[instrument(skip(self, stream))]
    pub async fn handle_websocket_stream(
        &self,
        stream: tokio_tungstenite::WebSocketStream<hyper::upgrade::Upgraded>,
    ) -> Result<()> {
        let (mut ws_sender, mut ws_receiver) = stream.split();

        let connection_id = Uuid::new_v4();
        let (tx, mut rx) = mpsc::unbounded_channel();

        info!("New WebSocket connection established: {}", connection_id);

        // Store the sender for this connection
        self.registry.register_connection(connection_id, tx).await?;

        // Clone service for message handling
        let service = self.clone();

        // Handle incoming messages
        let incoming_handle = tokio::spawn(async move {
            while let Some(msg) = ws_receiver.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Err(e) = service.handle_websocket_message(connection_id, text).await
                        {
                            error!("Error handling WebSocket message: {}", e);
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("WebSocket connection closed: {}", connection_id);
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });

        // Handle outgoing messages
        let outgoing_handle = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                let json_message = match serde_json::to_string(&message) {
                    Ok(json) => json,
                    Err(e) => {
                        error!("Failed to serialize message: {}", e);
                        continue;
                    }
                };

                if let Err(e) = ws_sender.send(Message::Text(json_message)).await {
                    error!("Failed to send WebSocket message: {}", e);
                    break;
                }
            }
        });

        // Wait for either handle to complete
        tokio::select! {
            _ = incoming_handle => {},
            _ = outgoing_handle => {},
        }

        // Clean up connection and associated services
        if let Err(e) = self.registry.remove_connection(connection_id).await {
            error!("Failed to clean up connection {}: {}", connection_id, e);
        }

        info!("WebSocket connection ended: {}", connection_id);
        Ok(())
    }

    #[instrument(skip(self, message))]
    pub async fn handle_websocket_message(
        &self,
        connection_id: Uuid,
        message: String,
    ) -> Result<()> {
        // First, try to parse as IngressMessage to intercept WS proxy messages
        if let Ok(parsed) = serde_json::from_str::<crate::common::IngressMessage>(&message) {
            match parsed {
                crate::common::IngressMessage::WebSocketProxyInitAck {
                    session_id,
                    success,
                    message,
                    response_headers,
                } => {
                    self.handle_ws_proxy_init_ack(session_id, success, message, response_headers)
                        .await;
                    return Ok(());
                }
                crate::common::IngressMessage::WebSocketProxyData {
                    session_id,
                    frame_type,
                    payload,
                } => {
                    self.handle_ws_proxy_data_from_agent(session_id, frame_type, payload)
                        .await;
                    return Ok(());
                }
                crate::common::IngressMessage::WebSocketProxyClose {
                    session_id,
                    code,
                    reason,
                } => {
                    self.handle_ws_proxy_close_from_agent(session_id, code, reason)
                        .await;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Fallback to standard dispatcher for other messages
        match self
            .dispatcher
            .handle_message(
                connection_id,
                message,
                self.auth_service.as_ref(),
                self.registry.as_ref(),
                self.router.as_ref(),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
