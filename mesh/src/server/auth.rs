use crate::common::auth::{IamAuthRequest, IamAuthResponse, IamIdentity};
use async_trait::async_trait;
use hyper::Client as HttpClient;
use hyper_tls::HttpsConnector;
use tracing::{info, warn};

/// Service for handling IAM authentication
#[async_trait]
pub trait AuthService: Send + Sync {
    /// Authenticate using IAM presigned URL or skip validation if configured
    async fn authenticate(&self, auth_request: &IamAuthRequest) -> IamAuthResponse;
}

/// Default implementation of AuthService
pub struct DefaultAuthService {
    http_client: HttpClient<HttpsConnector<hyper::client::HttpConnector>>,
    allowed_role_patterns: Vec<String>,
    skip_validation: bool,
}

impl DefaultAuthService {
    pub fn new(
        http_client: HttpClient<HttpsConnector<hyper::client::HttpConnector>>,
        allowed_role_patterns: Vec<String>,
        skip_validation: bool,
    ) -> Self {
        Self {
            http_client,
            allowed_role_patterns,
            skip_validation,
        }
    }

    /// Extract XML field from STS response
    fn extract_xml_field(xml: &str, tag: &str) -> Option<String> {
        let start = format!("<{}>", tag);
        let end = format!("</{}>", tag);
        let si = xml.find(&start)? + start.len();
        let ei = xml[si..].find(&end)? + si;
        Some(xml[si..ei].to_string())
    }

    /// Check if an ARN matches any of the allowed patterns
    fn is_role_allowed(&self, arn: &str) -> bool {
        if self.allowed_role_patterns.is_empty()
            || self.allowed_role_patterns.contains(&"*".to_string())
        {
            return true;
        }

        self.allowed_role_patterns
            .iter()
            .any(|pattern| self.matches_arn_pattern(arn, pattern))
    }

    /// Check if an ARN matches a pattern (supports * wildcards)
    fn matches_arn_pattern(&self, arn: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        if !pattern.contains('*') {
            return arn == pattern;
        }

        // Simple wildcard matching
        let pattern_parts: Vec<&str> = pattern.split('*').collect();
        if pattern_parts.is_empty() {
            return false;
        }

        let mut arn_remaining = arn;

        for (i, part) in pattern_parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }

            if i == 0 && !arn_remaining.starts_with(part) {
                return false;
            }

            if let Some(pos) = arn_remaining.find(part) {
                if i == 0 && pos != 0 {
                    return false;
                }
                arn_remaining = &arn_remaining[pos + part.len()..];
            } else {
                return false;
            }
        }

        pattern.ends_with('*') || arn_remaining.is_empty()
    }

    /// Validate IAM identity using presigned STS URL
    async fn validate_with_sts(&self, presigned_url: &str, _region: &str) -> IamAuthResponse {
        match presigned_url.parse() {
            Ok(uri) => match self.http_client.get(uri).await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = hyper::body::to_bytes(resp.into_body())
                        .await
                        .unwrap_or_default();

                    if status.is_success() {
                        let body_str = String::from_utf8_lossy(&body);
                        let arn = Self::extract_xml_field(&body_str, "Arn");
                        let account = Self::extract_xml_field(&body_str, "Account");
                        let user_id = Self::extract_xml_field(&body_str, "UserId");

                        if let (Some(arn), Some(account), Some(user_id)) = (arn, account, user_id) {
                            if self.is_role_allowed(&arn) {
                                info!("IAM auth successful for ARN: {}", arn);
                                IamAuthResponse {
                                    success: true,
                                    error: None,
                                    identity: Some(IamIdentity {
                                        arn,
                                        account_id: account,
                                        user_id,
                                        principal_type: "AssumedRole".to_string(),
                                    }),
                                }
                            } else {
                                warn!("Role not allowed: {}", arn);
                                IamAuthResponse {
                                    success: false,
                                    error: Some("Role not allowed".to_string()),
                                    identity: None,
                                }
                            }
                        } else {
                            warn!("Failed to parse STS identity from response");
                            IamAuthResponse {
                                success: false,
                                error: Some("Failed to parse STS identity".to_string()),
                                identity: None,
                            }
                        }
                    } else {
                        warn!("STS call returned non-success status: {}", status);
                        IamAuthResponse {
                            success: false,
                            error: Some(format!("STS call failed with status: {}", status)),
                            identity: None,
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed STS presigned call: {}", e);
                    IamAuthResponse {
                        success: false,
                        error: Some(format!("STS call failed: {}", e)),
                        identity: None,
                    }
                }
            },
            Err(e) => {
                warn!("Invalid presigned URL format: {}", e);
                IamAuthResponse {
                    success: false,
                    error: Some(format!("Invalid presigned URL: {}", e)),
                    identity: None,
                }
            }
        }
    }
}

#[async_trait]
impl AuthService for DefaultAuthService {
    async fn authenticate(&self, auth_request: &IamAuthRequest) -> IamAuthResponse {
        if self.skip_validation {
            info!("IAM validation skipped (skip_validation=true)");
            return IamAuthResponse {
                success: true,
                error: None,
                identity: Some(IamIdentity {
                    arn: "arn:aws:iam::000000000000:role/skipped-validation".to_string(),
                    account_id: "000000000000".to_string(),
                    user_id: "skipped-validation".to_string(),
                    principal_type: "AssumedRole".to_string(),
                }),
            };
        }

        if let Some(presigned_url) = &auth_request.presigned_url {
            self.validate_with_sts(presigned_url, &auth_request.region)
                .await
        } else {
            warn!("IamAuth received without presigned_url");
            IamAuthResponse {
                success: false,
                error: Some("No presigned URL provided".to_string()),
                identity: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arn_pattern_matching() {
        let auth_service = DefaultAuthService::new(
            HttpClient::builder().build(HttpsConnector::new()),
            vec!["arn:aws:iam::*:role/MyRole".to_string()],
            false,
        );

        // Test exact match
        assert!(auth_service.matches_arn_pattern(
            "arn:aws:iam::123456789012:role/MyRole",
            "arn:aws:iam::123456789012:role/MyRole"
        ));

        // Test wildcard match
        assert!(auth_service.matches_arn_pattern(
            "arn:aws:iam::123456789012:role/MyRole",
            "arn:aws:iam::*:role/MyRole"
        ));

        // Test no match
        assert!(!auth_service.matches_arn_pattern(
            "arn:aws:iam::123456789012:role/MyRole",
            "arn:aws:iam::123456789012:role/OtherRole"
        ));
    }

    #[test]
    fn test_role_allowed() {
        let auth_service = DefaultAuthService::new(
            HttpClient::builder().build(HttpsConnector::new()),
            vec!["arn:aws:iam::*:role/MyRole".to_string()],
            false,
        );

        assert!(auth_service.is_role_allowed("arn:aws:iam::123456789012:role/MyRole"));
        assert!(!auth_service.is_role_allowed("arn:aws:iam::123456789012:role/OtherRole"));
    }

    #[test]
    fn test_wildcard_all_allowed() {
        let auth_service = DefaultAuthService::new(
            HttpClient::builder().build(HttpsConnector::new()),
            vec!["*".to_string()],
            false,
        );

        assert!(auth_service.is_role_allowed("arn:aws:iam::123456789012:role/AnyRole"));
    }
}
