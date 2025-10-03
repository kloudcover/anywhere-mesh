use hyper::{Body, Method, Request, Response, StatusCode};
use std::convert::Infallible;

use crate::server::CombinedIngressService;

impl CombinedIngressService {
    pub async fn handle_internal_request(
        &self,
        req: Request<Body>,
    ) -> Result<Response<Body>, Infallible> {
        match (req.method(), req.uri().path()) {
            (&Method::GET, "/health") => {
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

                let health_info = format!(
                    "{{\"status\":\"healthy\",\"connections\":{},\"registrations\":{},\"instance_id\":\"{}\",\"started_at\":{}}}",
                    connections_count, registrations_count, instance_id, started_at
                );

                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(health_info))
                    .unwrap())
            }
            (&Method::GET, "/metrics") => {
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

                let metrics = format!(
                    "# HELP connections_total Total number of WebSocket connections\n# TYPE connections_total gauge\nconnections_total {}\n# HELP registrations_total Total number of service registrations\n# TYPE registrations_total gauge\nregistrations_total {}\n",
                    connections_count,
                    registrations_count
                );

                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/plain")
                    .body(Body::from(metrics))
                    .unwrap())
            }
            _ => Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not Found"))
                .unwrap()),
        }
    }
}
