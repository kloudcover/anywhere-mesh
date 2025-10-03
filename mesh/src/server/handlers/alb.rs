use crate::common::ProxyRequest;
use anyhow::Result;
use hyper::{Body, Method, Request, Response, StatusCode};
use std::collections::HashMap;
use std::convert::Infallible;
use tracing::{error, info, instrument};
use uuid::Uuid;

use crate::server::CombinedIngressService;

impl CombinedIngressService {
    /// Filter headers to only include those needed for routing and proxying
    fn filter_proxy_headers(headers: &hyper::HeaderMap) -> HashMap<String, String> {
        let mut filtered = HashMap::new();

        // Whitelist of headers to forward (essential for routing/proxying)
        let whitelist = [
            "host",
            "user-agent",
            "accept",
            "accept-encoding",
            "accept-language",
            "authorization",
            "cookie",
            "x-forwarded-for",
            "x-forwarded-proto",
            "x-forwarded-host",
            "x-real-ip",
            "content-type",
            "content-length",
            "x-test-route", // For testing purposes
        ];

        for (name, value) in headers.iter() {
            let name_str = name.as_str().to_lowercase();
            if whitelist.contains(&name_str.as_str()) {
                if let Ok(value_str) = value.to_str() {
                    filtered.insert(name_str, value_str.to_string());
                }
            }
        }

        filtered
    }

    #[instrument(skip(self, req))]
    pub async fn handle_alb_request(
        &self,
        req: Request<Body>,
    ) -> Result<Response<Body>, Infallible> {
        // If this is a WebSocket upgrade and feature enabled, switch to ws proxy flow
        let is_ws_upgrade = req
            .headers()
            .get("upgrade")
            .and_then(|h| h.to_str().ok())
            .map(|h| h.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false);
        if is_ws_upgrade && Self::ws_proxy_enabled() {
            return self.start_ws_tunnel_from_alb(req).await;
        }
        // Handle health check on ALB port
        if req.method() == Method::GET && req.uri().path() == "/health" {
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

            let health_info = format!(
                "{{\"status\":\"healthy\",\"connections\":{},\"registrations\":{},\"port\":\"8080\"}}",
                connections_count, registrations_count
            );

            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(Body::from(health_info))
                .unwrap());
        }

        let host = req
            .headers()
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown");

        info!("Received ALB request for host: {}", host);

        match self.process_alb_request(req).await {
            Ok(response) => Ok(response),
            Err(e) => {
                error!("Error processing ALB request: {}", e);
                Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from(format!("Proxy error: {}", e)))
                    .unwrap())
            }
        }
    }

    #[instrument(skip(self, req))]
    async fn process_alb_request(&self, req: Request<Body>) -> Result<Response<Body>> {
        let (parts, body) = req.into_parts();
        let body_bytes = hyper::body::to_bytes(body).await?;

        // Extract host from headers/authority robustly (HTTP/1.1 and HTTP/2)
        let host = parts
            .headers
            .get("x-forwarded-host")
            .or_else(|| parts.headers.get("host"))
            .and_then(|h| h.to_str().ok())
            .map(|s| s.split(':').next().unwrap_or(s).to_string())
            .or_else(|| parts.uri.authority().map(|a| a.host().to_string()))
            .unwrap_or_else(|| "unknown".to_string());

        // Filter and convert headers to HashMap (only essential headers)
        let mut headers = Self::filter_proxy_headers(&parts.headers);
        // Ensure proto is set to https when behind ALB TLS termination
        headers.insert("x-forwarded-proto".to_string(), "https".to_string());

        // Create proxy request
        let proxy_request = ProxyRequest {
            id: Uuid::new_v4(),
            method: parts.method.to_string(),
            path: parts
                .uri
                .path_and_query()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "/".to_string()),
            headers,
            body: if body_bytes.is_empty() {
                None
            } else {
                Some(body_bytes.to_vec())
            },
            target_host: host,
        };

        // Route request directly through WebSocket connections
        match self.route_request_through_websocket(proxy_request).await {
            Ok(response) => {
                // Build HTTP response
                let mut response_builder = Response::builder().status(response.status_code);

                // Add headers (preserve duplicates like Set-Cookie)
                for (name, value) in response.headers {
                    response_builder = response_builder.header(name.as_str(), value.as_str());
                }

                let response_body = response.body.unwrap_or_default();
                Ok(response_builder.body(Body::from(response_body))?)
            }
            Err(e) => {
                error!("Error routing request: {}", e);
                Ok(Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Body::from("Service Unavailable"))?)
            }
        }
    }
}
