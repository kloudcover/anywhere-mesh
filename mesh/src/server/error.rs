use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur in the ingress service
#[derive(Error, Debug)]
pub enum IngressError {
    #[error("Bad request: {message}")]
    BadRequest { message: String },

    #[error("Request timeout for request {request_id}")]
    Timeout { request_id: Uuid },

    #[error("Registry entry not found for {entity_id}")]
    RegistryNotFound { entity_id: Uuid },

    #[error("Failed to send message: {message}")]
    SendFailed { message: String },

    #[error("Serialization/deserialization error: {source}")]
    Serde {
        #[from]
        source: serde_json::Error,
    },

    #[error("HTTP client error: {source}")]
    Http {
        #[from]
        source: hyper::Error,
    },

    #[error("WebSocket error: {source}")]
    WebSocket {
        #[from]
        source: Box<tokio_tungstenite::tungstenite::Error>,
    },

    #[error("Internal error: {message}")]
    Internal { message: String },
}

impl IngressError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest {
            message: message.into(),
        }
    }

    pub fn timeout(request_id: Uuid) -> Self {
        Self::Timeout { request_id }
    }

    pub fn registry_not_found(entity_id: Uuid) -> Self {
        Self::RegistryNotFound { entity_id }
    }

    pub fn send_failed(message: impl Into<String>) -> Self {
        Self::SendFailed {
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

pub type IngressResult<T> = Result<T, IngressError>;
