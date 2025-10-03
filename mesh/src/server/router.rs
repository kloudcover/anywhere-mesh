use super::error::{IngressError, IngressResult};
use super::registry::Registry;
use crate::common::{routing, IngressMessage, ProxyRequest, ProxyResponse, ServiceRegistration};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, RwLock};
use tokio::time::timeout;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Cache entry for host→service mappings
#[derive(Clone)]
struct HostServiceCacheEntry {
    services: Vec<ServiceRegistration>,
    timestamp: Instant,
}

/// Service for routing requests and managing responses
#[async_trait]
pub trait Router: Send + Sync {
    /// Route a proxy request to an appropriate service and wait for response
    async fn route_request(
        &self,
        proxy_request: ProxyRequest,
        registry: &dyn Registry,
    ) -> IngressResult<ProxyResponse>;

    /// Handle an incoming proxy response by matching it to a pending request
    async fn handle_response(&self, response: ProxyResponse) -> IngressResult<()>;
}

/// Default implementation of Router
#[derive(Clone)]
pub struct DefaultRouter {
    pending_requests: Arc<RwLock<HashMap<Uuid, oneshot::Sender<ProxyResponse>>>>,
    request_timeout: Duration,
    host_service_cache: Arc<RwLock<HashMap<String, HostServiceCacheEntry>>>,
    cache_ttl: Duration,
}

impl DefaultRouter {
    pub fn new(request_timeout: Duration) -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            request_timeout,
            host_service_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: Duration::from_secs(30), // 30 second TTL
        }
    }

    /// Find services matching the target host (with caching)
    async fn find_matching_services(
        &self,
        target_host: &str,
        registry: &dyn Registry,
    ) -> IngressResult<Vec<ServiceRegistration>> {
        // Check cache first
        {
            let cache = self.host_service_cache.read().await;
            if let Some(entry) = cache.get(target_host) {
                if entry.timestamp.elapsed() < self.cache_ttl {
                    debug!("Cache hit for host: {}", target_host);
                    return Ok(entry.services.clone());
                }
            }
        }

        debug!("Cache miss for host: {}, computing services", target_host);

        // Cache miss or expired - compute matching services
        let registrations = registry.get_all_registrations().await?;

        let matching_services: Vec<_> = registrations
            .values()
            .filter(|reg| routing::match_host_to_service(target_host, &[(*reg).clone()]).is_some())
            .cloned()
            .collect();

        // Cache the result
        {
            let mut cache = self.host_service_cache.write().await;
            cache.insert(
                target_host.to_string(),
                HostServiceCacheEntry {
                    services: matching_services.clone(),
                    timestamp: Instant::now(),
                },
            );
        }

        Ok(matching_services)
    }

    /// Select a healthy service from matching services
    async fn select_healthy_service(
        &self,
        matching_services: &[ServiceRegistration],
        registry: &dyn Registry,
    ) -> IngressResult<Option<ServiceRegistration>> {
        let connections = registry.get_all_connections().await?;
        let selected = routing::select_healthy_instance(matching_services, &connections);
        Ok(selected.cloned())
    }

    /// Forward a request to a service and set up response waiting
    async fn forward_request(
        &self,
        proxy_request: &ProxyRequest,
        service: &ServiceRegistration,
        registry: &dyn Registry,
    ) -> IngressResult<oneshot::Receiver<ProxyResponse>> {
        // Create a oneshot channel for the response
        let (response_tx, response_rx) = oneshot::channel();

        // Store the response sender
        {
            let mut pending_requests = self.pending_requests.write().await;
            pending_requests.insert(proxy_request.id, response_tx);
        }

        // Get the connection sender and forward the request
        if let Some(sender) = registry.get_connection_sender(service.id).await? {
            let forward_message = IngressMessage::ProxyRequestForward(proxy_request.clone());

            if let Err(e) = sender.send(forward_message) {
                // Clean up pending request on send failure
                let mut pending_requests = self.pending_requests.write().await;
                pending_requests.remove(&proxy_request.id);

                return Err(IngressError::send_failed(format!(
                    "Failed to send request to service: {}",
                    e
                )));
            }

            Ok(response_rx)
        } else {
            // Clean up pending request if no sender found
            let mut pending_requests = self.pending_requests.write().await;
            pending_requests.remove(&proxy_request.id);

            Err(IngressError::send_failed(
                "No connection sender found for service",
            ))
        }
    }

    /// Wait for response with timeout and handle cleanup
    async fn wait_for_response(
        &self,
        proxy_request: &ProxyRequest,
        response_rx: oneshot::Receiver<ProxyResponse>,
    ) -> IngressResult<ProxyResponse> {
        match timeout(self.request_timeout, response_rx).await {
            Ok(Ok(response)) => {
                debug!("Received response for request: {}", proxy_request.id);
                Ok(response)
            }
            Ok(Err(_)) => {
                warn!("Response channel closed for request: {}", proxy_request.id);
                // Clean up pending request
                let mut pending_requests = self.pending_requests.write().await;
                pending_requests.remove(&proxy_request.id);

                Err(IngressError::internal("Response channel closed"))
            }
            Err(_) => {
                warn!("Request timeout for request: {}", proxy_request.id);
                // Clean up pending request on timeout
                let mut pending_requests = self.pending_requests.write().await;
                pending_requests.remove(&proxy_request.id);

                Err(IngressError::timeout(proxy_request.id))
            }
        }
    }

    /// Create error response for various failure scenarios
    fn create_error_response(request_id: Uuid, status_code: u16, message: &str) -> ProxyResponse {
        ProxyResponse {
            id: request_id,
            status_code,
            headers: Vec::new(),
            body: Some(message.as_bytes().to_vec()),
        }
    }
}

#[async_trait]
impl Router for DefaultRouter {
    async fn route_request(
        &self,
        proxy_request: ProxyRequest,
        registry: &dyn Registry,
    ) -> IngressResult<ProxyResponse> {
        // Find matching services for the host
        let matching_services = self
            .find_matching_services(&proxy_request.target_host, registry)
            .await?;

        if matching_services.is_empty() {
            warn!(
                "No matching services found for host: {}",
                proxy_request.target_host
            );
            return Ok(Self::create_error_response(
                proxy_request.id,
                404,
                "Service Not Found",
            ));
        }

        debug!(
            "Found {} matching services for host: {}",
            matching_services.len(),
            proxy_request.target_host
        );

        // Select a healthy instance
        let selected_service = self
            .select_healthy_service(&matching_services, registry)
            .await?;

        debug!(
            "Health check result: {}",
            if selected_service.is_some() {
                "healthy service found"
            } else {
                "no healthy services"
            }
        );

        if let Some(service) = selected_service {
            info!("Routing request to service: {}", service.service_name);

            // Forward request and wait for response
            match self
                .forward_request(&proxy_request, &service, registry)
                .await
            {
                Ok(response_rx) => {
                    // Wait for response with timeout
                    match self.wait_for_response(&proxy_request, response_rx).await {
                        Ok(response) => Ok(response),
                        Err(IngressError::Timeout { .. }) => Ok(Self::create_error_response(
                            proxy_request.id,
                            504,
                            "Gateway Timeout",
                        )),
                        Err(_) => Ok(Self::create_error_response(
                            proxy_request.id,
                            503,
                            "Service Unavailable",
                        )),
                    }
                }
                Err(_) => Ok(Self::create_error_response(
                    proxy_request.id,
                    503,
                    "Service Unavailable",
                )),
            }
        } else {
            Ok(Self::create_error_response(
                proxy_request.id,
                503,
                "No healthy service available",
            ))
        }
    }

    async fn handle_response(&self, response: ProxyResponse) -> IngressResult<()> {
        debug!("Received proxy response for request: {}", response.id);

        // Find the pending request and send the response
        let mut pending_requests = self.pending_requests.write().await;
        if let Some(response_tx) = pending_requests.remove(&response.id) {
            if response_tx.send(response).is_err() {
                warn!("Failed to send response to pending request (receiver dropped)");
            }
            Ok(())
        } else {
            warn!("Received response for unknown request: {}", response.id);
            Err(IngressError::registry_not_found(response.id))
        }
    }
}

impl Default for DefaultRouter {
    fn default() -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            request_timeout: Duration::from_secs(30),
            host_service_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: Duration::from_secs(30),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ServiceRegistration;
    use crate::server::registry::{DefaultRegistry, Registry};

    #[tokio::test]
    async fn test_route_request_no_services() {
        let router = DefaultRouter::new(Duration::from_secs(1));
        let registry = DefaultRegistry::new();

        let request = ProxyRequest {
            id: Uuid::new_v4(),
            method: "GET".to_string(),
            path: "/test".to_string(),
            headers: HashMap::new(),
            body: None,
            target_host: "nonexistent.example.com".to_string(),
        };

        let response = router.route_request(request, &registry).await.unwrap();
        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_route_request_no_healthy_services() {
        let router = DefaultRouter::new(Duration::from_secs(1));
        let registry = DefaultRegistry::new();
        let connection_id = Uuid::new_v4();

        // Register a service but no connection (unhealthy)
        let registration = ServiceRegistration {
            id: connection_id,
            service_name: "test-service".to_string(),
            host: "test.example.com".to_string(),
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

        let request = ProxyRequest {
            id: Uuid::new_v4(),
            method: "GET".to_string(),
            path: "/test".to_string(),
            headers: HashMap::new(),
            body: None,
            target_host: "test.example.com".to_string(),
        };

        let response = router.route_request(request, &registry).await.unwrap();
        assert_eq!(response.status_code, 503);
    }

    #[tokio::test]
    async fn test_handle_response() {
        let router = DefaultRouter::new(Duration::from_secs(1));
        let request_id = Uuid::new_v4();

        // Simulate a pending request
        let (tx, _rx) = oneshot::channel();
        {
            let mut pending = router.pending_requests.write().await;
            pending.insert(request_id, tx);
        }

        let response = ProxyResponse {
            id: request_id,
            status_code: 200,
            headers: Vec::new(),
            body: Some(b"test response".to_vec()),
        };

        let result = router.handle_response(response).await;
        assert!(result.is_ok());

        // Verify request was removed from pending
        let pending = router.pending_requests.read().await;
        assert!(!pending.contains_key(&request_id));
    }

    #[tokio::test]
    async fn test_handle_response_unknown_request() {
        let router = DefaultRouter::new(Duration::from_secs(1));
        let request_id = Uuid::new_v4();

        let response = ProxyResponse {
            id: request_id,
            status_code: 200,
            headers: Vec::new(),
            body: Some(b"test response".to_vec()),
        };

        let result = router.handle_response(response).await;
        assert!(result.is_err());
    }
}
