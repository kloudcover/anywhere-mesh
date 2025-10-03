pub mod ws;
use crate::common::{ProxyRequest, ProxyResponse};
use anyhow::Result;
use hyper::{Body, Client, Method, Request, Uri};
use hyper_tls::HttpsConnector;

use std::str::FromStr;
use tracing::{debug, error, info, instrument, warn};

#[derive(Clone)]
pub struct ProxyHandler {
    http_client: Client<HttpsConnector<hyper::client::HttpConnector>>,
    local_endpoint: String,
}

impl ProxyHandler {
    pub fn new(
        http_client: Client<HttpsConnector<hyper::client::HttpConnector>>,
        local_endpoint: String,
    ) -> Self {
        Self {
            http_client,
            local_endpoint,
        }
    }

    #[instrument(skip(self, proxy_request))]
    pub async fn handle_request(&self, proxy_request: ProxyRequest) -> ProxyResponse {
        debug!(
            "Handling proxy request {} to {}{}",
            proxy_request.id, self.local_endpoint, proxy_request.path
        );

        match self.forward_request(proxy_request.clone()).await {
            Ok(response) => {
                debug!("Successfully forwarded request {}", proxy_request.id);
                response
            }
            Err(e) => {
                error!("Failed to forward request {}: {}", proxy_request.id, e);
                ProxyResponse {
                    id: proxy_request.id,
                    status_code: 500,
                    headers: Vec::new(),
                    body: Some(format!("Internal Server Error: {}", e).into_bytes()),
                }
            }
        }
    }

    async fn forward_request(&self, proxy_request: ProxyRequest) -> Result<ProxyResponse> {
        // Construct the full URL
        let url = format!("{}{}", self.local_endpoint, proxy_request.path);
        let uri = Uri::from_str(&url)?;

        // Parse the HTTP method
        let method = Method::from_str(&proxy_request.method)?;

        // Build the request
        let mut request_builder = Request::builder().method(method).uri(uri);

        // Add headers
        for (name, value) in &proxy_request.headers {
            request_builder = request_builder.header(name, value);
        }

        // Add body if present
        let body = if let Some(body_bytes) = proxy_request.body {
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        let request = request_builder.body(body)?;

        info!("Forwarding {} request to {}", proxy_request.method, url);

        // Send the request
        let response = self.http_client.request(request).await?;

        // Extract response data
        let status_code = response.status().as_u16();
        let mut headers: Vec<(String, String)> = Vec::new();

        // Preserve all headers including duplicates (e.g., multiple Set-Cookie)
        for (name, value) in response.headers().iter() {
            if let Ok(value_str) = value.to_str() {
                headers.push((name.to_string(), value_str.to_string()));
            }
        }

        // Read response body
        let body_bytes = hyper::body::to_bytes(response.into_body()).await?;
        let body = if body_bytes.is_empty() {
            None
        } else {
            Some(body_bytes.to_vec())
        };

        debug!(
            "Received response with status {} for request {}",
            status_code, proxy_request.id
        );

        Ok(ProxyResponse {
            id: proxy_request.id,
            status_code,
            headers,
            body,
        })
    }

    pub async fn health_check(&self, health_check_path: &str) -> Result<bool> {
        let url = format!("{}{}", self.local_endpoint, health_check_path);

        debug!("Performing health check: {}", url);

        match self.http_client.get(url.parse()?).await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    debug!("Health check passed: {}", status);
                    Ok(true)
                } else {
                    warn!("Health check failed with status: {}", status);
                    Ok(false)
                }
            }
            Err(e) => {
                error!("Health check request failed: {}", e);
                Ok(false)
            }
        }
    }
}
