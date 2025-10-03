use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::{IamAuthRequest, IamAuthResponse};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RouteConfig {
    pub host: String,
    pub target_service: String,
    pub ecs_cluster: String,
    pub ecs_service: String,
    pub attributes: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRegistration {
    pub id: Uuid,
    pub host: String,
    pub port: u16,
    pub service_name: String,
    pub cluster_name: String,
    pub task_arn: String,
    pub attributes: HashMap<String, String>,
    pub health_check_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRequest {
    pub id: Uuid,
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
    pub target_host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyResponse {
    pub id: Uuid,
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IngressMessage {
    // From Anywhere Mesh cluster client
    HeartBeat {
        cluster_name: String,
        client_id: Uuid,
    },
    ProxyResponse(ProxyResponse),

    // From ALB proxy service
    ProxyRequest(ProxyRequest),

    // From ingress service
    ProxyRequestForward(ProxyRequest),

    // WebSocket proxying (ALB <-> agent)
    WebSocketProxyInit {
        session_id: Uuid,
        target_host: String,
        path: String,
        headers: HashMap<String, String>,
        subprotocols: Option<Vec<String>>,
    },
    WebSocketProxyInitAck {
        session_id: Uuid,
        success: bool,
        message: Option<String>,
        response_headers: Option<HashMap<String, String>>,
    },
    WebSocketProxyData {
        session_id: Uuid,
        // "text" | "binary" | "ping" | "pong"
        frame_type: String,
        // base64 for binary; utf8 for text; empty for control frames
        payload: Option<String>,
    },
    WebSocketProxyClose {
        session_id: Uuid,
        code: Option<u16>,
        reason: Option<String>,
    },

    // Legacy messages (for backward compatibility during transition)
    IamAuth(IamAuthRequest),
    ServiceRegistration(ServiceRegistration),
    ServiceDeregistration {
        id: Uuid,
    },
    IamAuthResponse(IamAuthResponse),
    RegistrationAck {
        id: Uuid,
        success: bool,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: Uuid,
    pub service_name: String,
    pub host: String,
    pub port: u16,
    pub last_heartbeat: std::time::SystemTime,
    pub attributes: HashMap<String, String>,
}
