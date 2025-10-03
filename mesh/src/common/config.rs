use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for the ingress service
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct IngressConfig {
    /// Server configuration
    pub server: ServerConfig,

    /// ECS cluster-based authentication configuration
    pub ecs: EcsConfig,

    /// Routing configuration
    pub routing: RoutingConfig,

    /// Logging configuration
    pub logging: LoggingConfig,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ServerConfig {
    /// Port for ALB traffic
    #[serde(default = "default_alb_port")]
    pub alb_port: u16,

    /// Port for health checks and metrics
    #[serde(default = "default_health_port")]
    pub health_port: u16,

    /// Port for WebSocket connections from clients
    #[serde(default = "default_websocket_port")]
    pub websocket_port: u16,

    /// Request timeout in seconds
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    /// Maximum number of concurrent connections
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

/// ECS cluster-based authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct EcsConfig {
    /// List of allowed ECS cluster ARNs (supports wildcards)
    pub allowed_clusters: Vec<String>,

    /// AWS region for ECS operations
    #[serde(default = "default_aws_region")]
    pub region: String,

    /// Whether to skip cluster validation (for development)
    #[serde(default)]
    pub skip_validation: bool,

    /// Service discovery interval in seconds
    #[serde(default = "default_discovery_interval")]
    pub discovery_interval: u64,

    /// Required labels for service discovery
    pub required_labels: Vec<String>,
}

/// Routing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RoutingConfig {
    /// Health check interval in seconds
    #[serde(default = "default_health_check_interval")]
    pub health_check_interval: u64,

    /// Maximum time since last heartbeat before marking unhealthy (seconds)
    #[serde(default = "default_unhealthy_threshold")]
    pub unhealthy_threshold: u64,

    /// Load balancing strategy
    #[serde(default)]
    pub load_balancing: LoadBalancingStrategy,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct LoggingConfig {
    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Whether to log in JSON format
    #[serde(default)]
    pub json_format: bool,

    /// Whether to include request/response bodies in logs
    #[serde(default)]
    pub log_bodies: bool,
}

/// Load balancing strategies
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum LoadBalancingStrategy {
    #[default]
    RoundRobin,
    Random,
    LeastConnections,
}

/// Configuration for the Anywhere Mesh cluster client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ClientConfig {
    /// Connection configuration
    pub connection: ConnectionConfig,

    /// ECS cluster configuration
    pub cluster: ClusterConfig,

    /// AWS configuration
    pub aws: AwsConfig,

    /// Logging configuration
    pub logging: LoggingConfig,
}

/// Connection configuration for client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ConnectionConfig {
    /// Ingress service WebSocket endpoint
    pub ingress_endpoint: String,

    /// Local service endpoint to proxy to
    pub local_endpoint: String,

    /// Connection retry configuration
    pub retry: RetryConfig,

    /// Heartbeat interval in seconds
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval: u64,
}

/// ECS cluster configuration for the client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ClusterConfig {
    /// ECS cluster name to monitor
    pub cluster_name: String,

    /// Service discovery interval in seconds
    #[serde(default = "default_discovery_interval")]
    pub discovery_interval: u64,

    /// Default local port for services without explicit port labels
    #[serde(default = "default_local_port")]
    pub default_local_port: u16,

    /// Local host where services are running
    #[serde(default = "default_local_host")]
    pub local_host: String,

    /// Labels to filter tasks for service discovery
    #[serde(default)]
    pub label_filters: HashMap<String, String>,
}

/// AWS configuration for client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct AwsConfig {
    /// AWS region
    #[serde(default = "default_aws_region")]
    pub region: String,

    /// Whether to skip IAM validation (for development)
    #[serde(default)]
    pub skip_iam_validation: bool,

    /// Profile to use for AWS credentials
    pub profile: Option<String>,
}

/// Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    #[serde(default = "default_max_retries")]
    pub max_attempts: u32,

    /// Initial retry delay in milliseconds
    #[serde(default = "default_initial_delay")]
    pub initial_delay_ms: u64,

    /// Maximum retry delay in milliseconds
    #[serde(default = "default_max_delay")]
    pub max_delay_ms: u64,

    /// Backoff multiplier
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

// Default value functions
#[allow(dead_code)]
fn default_alb_port() -> u16 {
    8080
}
#[allow(dead_code)]
fn default_health_port() -> u16 {
    8081
}
#[allow(dead_code)]
fn default_websocket_port() -> u16 {
    8082
}
#[allow(dead_code)]
fn default_request_timeout() -> u64 {
    30
}
#[allow(dead_code)]
fn default_max_connections() -> usize {
    1000
}
#[allow(dead_code)]
fn default_aws_region() -> String {
    "us-east-1".to_string()
}

#[allow(dead_code)]
fn default_discovery_interval() -> u64 {
    30
}
#[allow(dead_code)]
fn default_health_check_interval() -> u64 {
    30
}
#[allow(dead_code)]
fn default_unhealthy_threshold() -> u64 {
    90
}
#[allow(dead_code)]
fn default_log_level() -> String {
    "info".to_string()
}
#[allow(dead_code)]
fn default_heartbeat_interval() -> u64 {
    10
}

#[allow(dead_code)]
fn default_max_retries() -> u32 {
    5
}
#[allow(dead_code)]
fn default_initial_delay() -> u64 {
    1000
}
#[allow(dead_code)]
fn default_max_delay() -> u64 {
    30000
}
#[allow(dead_code)]
fn default_backoff_multiplier() -> f64 {
    2.0
}
#[allow(dead_code)]
fn default_local_port() -> u16 {
    3000
}
#[allow(dead_code)]
fn default_local_host() -> String {
    "localhost".to_string()
}
