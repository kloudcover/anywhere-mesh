use clap::Parser;

#[derive(Parser, Debug, Clone)]
pub struct ServerCommand {
    /// Port to listen on for HTTP requests from ALB
    #[arg(short, long, default_value = "8080")]
    pub alb_port: u16,

    /// Port to listen on for WebSocket connections
    #[arg(short, long, default_value = "8082")]
    pub websocket_port: u16,

    /// Request timeout in seconds
    #[arg(long, default_value = "30")]
    pub request_timeout: u64,
}

#[derive(Parser, Debug, Clone)]
pub struct ClientCommand {
    /// Ingress service WebSocket endpoint
    #[arg(
        short,
        long,
        env = "INGRESS_ENDPOINT",
        default_value = "ws://localhost:8082"
    )]
    pub ingress_endpoint: String,

    /// Local service endpoint to proxy to
    #[arg(
        short,
        long,
        env = "LOCAL_ENDPOINT",
        default_value = "http://localhost:3000"
    )]
    pub local_endpoint: String,

    /// Host to register for routing
    #[arg(long, env = "HOST", default_value = "localhost")]
    pub host: String,

    /// Port to register for the service
    #[arg(short, long, env = "PORT", default_value = "3000")]
    pub port: u16,

    /// Service name
    #[arg(long, env = "SERVICE_NAME", default_value = "my-service")]
    pub service_name: String,

    /// ECS cluster name
    #[arg(long, env = "CLUSTER_NAME", default_value = "my-cluster")]
    pub cluster_name: String,

    /// Health check path
    #[arg(long, default_value = "/health")]
    pub health_check_path: String,

    /// Skip IAM validation (for development)
    #[arg(long, env = "SKIP_IAM_VALIDATION")]
    pub skip_iam_validation: bool,
}
