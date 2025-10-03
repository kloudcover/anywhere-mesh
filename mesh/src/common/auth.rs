use serde::{Deserialize, Serialize};

/// IAM authentication request from client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IamAuthRequest {
    /// Optional presigned URL for STS GetCallerIdentity
    pub presigned_url: Option<String>,

    /// AWS region used for signing
    pub region: String,

    /// Optional identity payload (used when client cannot presign)
    pub arn: Option<String>,
    pub account_id: Option<String>,
    pub user_id: Option<String>,
}

/// IAM authentication response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IamAuthResponse {
    /// Whether authentication was successful
    pub success: bool,

    /// Error message if authentication failed
    pub error: Option<String>,

    /// Validated IAM identity information
    pub identity: Option<IamIdentity>,
}

/// Validated IAM identity information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IamIdentity {
    /// User or role ARN
    pub arn: String,

    /// AWS account ID
    pub account_id: String,

    /// User ID or role session name
    pub user_id: String,

    /// Principal type (User, AssumedRole, etc.)
    pub principal_type: String,
}
