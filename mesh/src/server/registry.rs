use super::error::{IngressError, IngressResult};
use crate::common::{ConnectionInfo, IngressMessage, ServiceRegistration};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

/// Service for managing connections and service registrations
#[async_trait]
pub trait Registry: Send + Sync {
    /// Register a new connection and store its message sender
    async fn register_connection(
        &self,
        connection_id: Uuid,
        sender: mpsc::UnboundedSender<IngressMessage>,
    ) -> IngressResult<()>;

    /// Remove a connection and clean up all associated data
    async fn remove_connection(&self, connection_id: Uuid) -> IngressResult<()>;

    /// Register a service with connection and registration information
    async fn register_service(
        &self,
        connection_id: Uuid,
        registration: ServiceRegistration,
    ) -> IngressResult<()>;

    /// Deregister a service
    async fn deregister_service(&self, service_id: Uuid) -> IngressResult<()>;

    /// Update the last heartbeat time for a connection
    async fn update_heartbeat(&self, connection_id: Uuid) -> IngressResult<()>;

    /// Get a connection sender by connection ID
    async fn get_connection_sender(
        &self,
        connection_id: Uuid,
    ) -> IngressResult<Option<mpsc::UnboundedSender<IngressMessage>>>;

    /// Get all current connections (for health/stats endpoints)
    async fn get_all_connections(&self) -> IngressResult<HashMap<Uuid, ConnectionInfo>>;

    /// Get all current registrations (for health/stats endpoints)
    async fn get_all_registrations(&self) -> IngressResult<HashMap<Uuid, ServiceRegistration>>;
}

/// Default implementation of Registry
#[derive(Clone)]
pub struct DefaultRegistry {
    connections: Arc<RwLock<HashMap<Uuid, ConnectionInfo>>>,
    registrations: Arc<RwLock<HashMap<Uuid, ServiceRegistration>>>,
    connection_senders: Arc<RwLock<HashMap<Uuid, mpsc::UnboundedSender<IngressMessage>>>>,
}

impl DefaultRegistry {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            registrations: Arc::new(RwLock::new(HashMap::new())),
            connection_senders: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl Registry for DefaultRegistry {
    async fn register_connection(
        &self,
        connection_id: Uuid,
        sender: mpsc::UnboundedSender<IngressMessage>,
    ) -> IngressResult<()> {
        let mut senders = self.connection_senders.write().await;
        senders.insert(connection_id, sender);
        info!("Connection registered: {}", connection_id);
        Ok(())
    }

    async fn remove_connection(&self, connection_id: Uuid) -> IngressResult<()> {
        // Remove from all maps
        {
            let mut connections = self.connections.write().await;
            connections.remove(&connection_id);
        }
        {
            let mut registrations = self.registrations.write().await;
            registrations.remove(&connection_id);
        }
        {
            let mut senders = self.connection_senders.write().await;
            senders.remove(&connection_id);
        }

        info!("Connection removed: {}", connection_id);
        Ok(())
    }

    async fn register_service(
        &self,
        connection_id: Uuid,
        registration: ServiceRegistration,
    ) -> IngressResult<()> {
        info!(
            "Service registration received: {}",
            registration.service_name
        );

        // Update connection info
        {
            let mut connections = self.connections.write().await;
            connections.insert(
                connection_id,
                ConnectionInfo {
                    id: connection_id,
                    service_name: registration.service_name.clone(),
                    host: registration.host.clone(),
                    port: registration.port,
                    last_heartbeat: SystemTime::now(),
                    attributes: registration.attributes.clone(),
                },
            );
        }

        // Store registration with connection_id
        {
            let mut registrations = self.registrations.write().await;
            let mut reg = registration.clone();
            reg.id = connection_id;
            registrations.insert(connection_id, reg);
        }

        Ok(())
    }

    async fn deregister_service(&self, service_id: Uuid) -> IngressResult<()> {
        info!("Service deregistration received for: {}", service_id);

        // Remove from registrations and connections
        {
            let mut registrations = self.registrations.write().await;
            registrations.remove(&service_id);
        }
        {
            let mut connections = self.connections.write().await;
            connections.remove(&service_id);
        }

        Ok(())
    }

    async fn update_heartbeat(&self, connection_id: Uuid) -> IngressResult<()> {
        let mut connections = self.connections.write().await;
        if let Some(connection) = connections.get_mut(&connection_id) {
            connection.last_heartbeat = SystemTime::now();
        } else {
            warn!(
                "Heartbeat received for unknown connection: {}",
                connection_id
            );
            return Err(IngressError::registry_not_found(connection_id));
        }
        Ok(())
    }

    async fn get_connection_sender(
        &self,
        connection_id: Uuid,
    ) -> IngressResult<Option<mpsc::UnboundedSender<IngressMessage>>> {
        let senders = self.connection_senders.read().await;
        Ok(senders.get(&connection_id).cloned())
    }

    async fn get_all_connections(&self) -> IngressResult<HashMap<Uuid, ConnectionInfo>> {
        let connections = self.connections.read().await;
        Ok(connections.clone())
    }

    async fn get_all_registrations(&self) -> IngressResult<HashMap<Uuid, ServiceRegistration>> {
        let registrations = self.registrations.read().await;
        Ok(registrations.clone())
    }
}

impl Default for DefaultRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ServiceRegistration;
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_connection_lifecycle() {
        let registry = DefaultRegistry::new();
        let connection_id = Uuid::new_v4();
        let (sender, _receiver) = mpsc::unbounded_channel();

        // Register connection
        registry
            .register_connection(connection_id, sender)
            .await
            .unwrap();

        // Verify connection exists
        let sender = registry.get_connection_sender(connection_id).await.unwrap();
        assert!(sender.is_some());

        // Remove connection
        registry.remove_connection(connection_id).await.unwrap();

        // Verify connection removed
        let sender = registry.get_connection_sender(connection_id).await.unwrap();
        assert!(sender.is_none());
    }

    #[tokio::test]
    async fn test_service_registration() {
        let registry = DefaultRegistry::new();
        let connection_id = Uuid::new_v4();

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

        // Register service
        registry
            .register_service(connection_id, registration.clone())
            .await
            .unwrap();

        // Verify service exists
        let all_registrations = registry.get_all_registrations().await.unwrap();
        assert_eq!(all_registrations.len(), 1);
        assert_eq!(
            all_registrations.values().next().unwrap().service_name,
            "test-service"
        );

        // Deregister service
        registry.deregister_service(connection_id).await.unwrap();

        // Verify service removed
        let all_registrations = registry.get_all_registrations().await.unwrap();
        assert_eq!(all_registrations.len(), 0);
    }
}
