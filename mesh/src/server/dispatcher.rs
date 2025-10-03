use super::auth::AuthService;
use super::error::{IngressError, IngressResult};
use super::registry::Registry;
use super::router::Router;
use crate::common::{IngressMessage, ProxyResponse};
use async_trait::async_trait;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Service for dispatching and handling WebSocket messages
#[async_trait]
pub trait MessageDispatcher: Send + Sync {
    /// Handle an incoming message from a WebSocket connection
    async fn handle_message(
        &self,
        connection_id: Uuid,
        message: String,
        auth_service: &dyn AuthService,
        registry: &dyn Registry,
        router: &dyn Router,
    ) -> IngressResult<()>;

    /// Send a response message to a specific connection
    async fn send_response(
        &self,
        connection_id: Uuid,
        response: IngressMessage,
        registry: &dyn Registry,
    ) -> IngressResult<()>;
}

/// Default implementation of MessageDispatcher
#[derive(Clone)]
pub struct DefaultMessageDispatcher;

impl DefaultMessageDispatcher {
    pub fn new() -> Self {
        Self
    }

    /// Parse and validate incoming message
    fn parse_message(&self, message: &str) -> IngressResult<IngressMessage> {
        serde_json::from_str(message)
            .map_err(|e| IngressError::bad_request(format!("Failed to parse message: {}", e)))
    }

    /// Handle IAM authentication messages
    async fn handle_iam_auth(
        &self,
        connection_id: Uuid,
        auth_request: crate::common::auth::IamAuthRequest,
        auth_service: &dyn AuthService,
        registry: &dyn Registry,
    ) -> IngressResult<()> {
        let response = auth_service.authenticate(&auth_request).await;

        self.send_response(
            connection_id,
            IngressMessage::IamAuthResponse(response),
            registry,
        )
        .await
    }

    /// Handle service registration messages
    async fn handle_service_registration(
        &self,
        connection_id: Uuid,
        registration: crate::common::ServiceRegistration,
        registry: &dyn Registry,
    ) -> IngressResult<()> {
        // Register the service with the registry
        if let Err(e) = registry
            .register_service(connection_id, registration.clone())
            .await
        {
            error!("Failed to register service: {}", e);
            // Send error acknowledgment
            let error_ack = IngressMessage::RegistrationAck {
                id: connection_id,
                success: false,
                message: format!("Registration failed: {}", e),
            };
            return self.send_response(connection_id, error_ack, registry).await;
        }

        // Send success acknowledgment
        let success_ack = IngressMessage::RegistrationAck {
            id: connection_id,
            success: true,
            message: "Service registered successfully".to_string(),
        };

        self.send_response(connection_id, success_ack, registry)
            .await
    }

    /// Handle heartbeat messages
    async fn handle_heartbeat(
        &self,
        connection_id: Uuid,
        cluster_name: String,
        registry: &dyn Registry,
    ) -> IngressResult<()> {
        debug!(
            "Received heartbeat from cluster: {} (conn {})",
            cluster_name, connection_id
        );

        if let Err(e) = registry.update_heartbeat(connection_id).await {
            warn!(
                "Failed to update heartbeat for connection {}: {}",
                connection_id, e
            );
            Err(e)
        } else {
            debug!("Updated heartbeat for connection: {}", connection_id);
            Ok(())
        }
    }

    /// Handle proxy response messages
    async fn handle_proxy_response(
        &self,
        response: ProxyResponse,
        router: &dyn Router,
    ) -> IngressResult<()> {
        if let Err(e) = router.handle_response(response).await {
            error!("Failed to handle proxy response: {}", e);
            Err(e)
        } else {
            Ok(())
        }
    }

    /// Handle service deregistration messages
    async fn handle_service_deregistration(
        &self,
        service_id: Uuid,
        registry: &dyn Registry,
    ) -> IngressResult<()> {
        if let Err(e) = registry.deregister_service(service_id).await {
            error!("Failed to deregister service {}: {}", service_id, e);
            Err(e)
        } else {
            info!("Service deregistered: {}", service_id);
            Ok(())
        }
    }
}

#[async_trait]
impl MessageDispatcher for DefaultMessageDispatcher {
    async fn handle_message(
        &self,
        connection_id: Uuid,
        message: String,
        auth_service: &dyn AuthService,
        registry: &dyn Registry,
        router: &dyn Router,
    ) -> IngressResult<()> {
        // Parse the incoming message
        let ingress_message = self.parse_message(&message)?;

        // Dispatch based on message type
        match ingress_message {
            IngressMessage::IamAuth(auth_request) => {
                self.handle_iam_auth(connection_id, auth_request, auth_service, registry)
                    .await
            }
            IngressMessage::ServiceRegistration(registration) => {
                self.handle_service_registration(connection_id, registration, registry)
                    .await
            }
            IngressMessage::HeartBeat { cluster_name, .. } => {
                self.handle_heartbeat(connection_id, cluster_name, registry)
                    .await
            }
            IngressMessage::ProxyResponse(response) => {
                self.handle_proxy_response(response, router).await
            }
            IngressMessage::ServiceDeregistration { id } => {
                self.handle_service_deregistration(id, registry).await
            }
            IngressMessage::WebSocketProxyInitAck {
                session_id,
                success,
                message,
                response_headers,
            } => {
                // Notify service/session manager
                // We cannot call async methods on service here directly; the service handles this elsewhere.
                // For now, just log; service will expose APIs to be called by outer layer if wired.
                // This dispatcher currently does not own a reference to CombinedIngressService.
                // The server/service will route this appropriately in the next wiring step.
                let _ = (session_id, success, message, response_headers);
                Ok(())
            }
            IngressMessage::WebSocketProxyData {
                session_id,
                frame_type,
                payload,
            } => {
                let _ = (session_id, frame_type, payload);
                Ok(())
            }
            IngressMessage::WebSocketProxyClose {
                session_id,
                code,
                reason,
            } => {
                let _ = (session_id, code, reason);
                Ok(())
            }
            _ => {
                warn!("Unexpected message type from Anywhere Mesh service");
                Err(IngressError::bad_request(
                    "Unexpected message type from Anywhere Mesh service",
                ))
            }
        }
    }

    async fn send_response(
        &self,
        connection_id: Uuid,
        response: IngressMessage,
        registry: &dyn Registry,
    ) -> IngressResult<()> {
        if let Some(sender) = registry.get_connection_sender(connection_id).await? {
            if let Err(e) = sender.send(response) {
                error!(
                    "Failed to send response to connection {}: {}",
                    connection_id, e
                );
                Err(IngressError::send_failed(format!(
                    "Failed to send response to connection: {}",
                    e
                )))
            } else {
                Ok(())
            }
        } else {
            warn!(
                "No sender found for connection {} when sending response",
                connection_id
            );
            Err(IngressError::registry_not_found(connection_id))
        }
    }
}

impl Default for DefaultMessageDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ServiceRegistration;
    use crate::server::registry::{DefaultRegistry, Registry};
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_parse_message_valid() {
        let dispatcher = DefaultMessageDispatcher::new();
        let heartbeat_json = r#"{"HeartBeat":{"cluster_name":"test","client_id":"00000000-0000-0000-0000-000000000000"}}"#;

        let result = dispatcher.parse_message(heartbeat_json);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_parse_message_invalid() {
        let dispatcher = DefaultMessageDispatcher::new();
        let invalid_json = "not valid json";

        let result = dispatcher.parse_message(invalid_json);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_heartbeat() {
        let dispatcher = DefaultMessageDispatcher::new();
        let registry = DefaultRegistry::new();
        let connection_id = Uuid::new_v4();

        // First register a service so heartbeat can find connection
        let registration = ServiceRegistration {
            id: connection_id,
            service_name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            cluster_name: "test-cluster".to_string(),
            task_arn: "arn:aws:ecs:us-east-1:123456789012:task/test-cluster/test-task".to_string(),
            health_check_path: Some("/health".to_string()),
            attributes: HashMap::new(),
        };

        registry
            .register_service(connection_id, registration)
            .await
            .unwrap();

        let result = dispatcher
            .handle_heartbeat(connection_id, "test-cluster".to_string(), &registry)
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_service_registration() {
        let dispatcher = DefaultMessageDispatcher::new();
        let registry = DefaultRegistry::new();
        let connection_id = Uuid::new_v4();
        let (sender, _receiver) = mpsc::unbounded_channel();

        // Register connection first
        registry
            .register_connection(connection_id, sender)
            .await
            .unwrap();

        let registration = ServiceRegistration {
            id: connection_id,
            service_name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            cluster_name: "test-cluster".to_string(),
            task_arn: "arn:aws:ecs:us-east-1:123456789012:task/test-cluster/test-task".to_string(),
            health_check_path: Some("/health".to_string()),
            attributes: HashMap::new(),
        };

        let result = dispatcher
            .handle_service_registration(connection_id, registration, &registry)
            .await;

        assert!(result.is_ok());

        // Verify service was registered
        let all_registrations = registry.get_all_registrations().await.unwrap();
        assert_eq!(all_registrations.len(), 1);
    }

    #[tokio::test]
    async fn test_handle_service_deregistration() {
        let dispatcher = DefaultMessageDispatcher::new();
        let registry = DefaultRegistry::new();
        let connection_id = Uuid::new_v4();

        // Register a service first
        let registration = ServiceRegistration {
            id: connection_id,
            service_name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            cluster_name: "test-cluster".to_string(),
            task_arn: "arn:aws:ecs:us-east-1:123456789012:task/test-cluster/test-task".to_string(),
            health_check_path: Some("/health".to_string()),
            attributes: HashMap::new(),
        };

        registry
            .register_service(connection_id, registration)
            .await
            .unwrap();

        // Deregister the service
        let result = dispatcher
            .handle_service_deregistration(connection_id, &registry)
            .await;

        assert!(result.is_ok());

        // Verify service was deregistered
        let all_registrations = registry.get_all_registrations().await.unwrap();
        assert_eq!(all_registrations.len(), 0);
    }

    #[tokio::test]
    async fn test_send_response_no_connection() {
        let dispatcher = DefaultMessageDispatcher::new();
        let registry = DefaultRegistry::new();
        let connection_id = Uuid::new_v4();

        let response = IngressMessage::RegistrationAck {
            id: connection_id,
            success: true,
            message: "test".to_string(),
        };

        let result = dispatcher
            .send_response(connection_id, response, &registry)
            .await;

        assert!(result.is_err());
    }
}
