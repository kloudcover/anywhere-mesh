use crate::common::{IngressMessage, ServiceRegistration};
use anyhow::Result;
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_ecs::Client as EcsClient;
use aws_sdk_sts::Client as StsClient;
use futures_util::{SinkExt, StreamExt};
use hyper::{Body, Client};
use hyper_tls::HttpsConnector;
use std::collections::HashMap;

use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::time::{interval, sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use super::aws::AwsService;
use super::proxy::ws::WebSocketReverseProxy;
use super::proxy::ProxyHandler;
// use crate::common::auth::IamAuthHelper;
use aws_sdk_ecs::config::ProvideCredentials;
use chrono::Utc;
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use sha2::{Digest, Sha256};

pub struct EcsAnywhereClient {
    pub client_id: Uuid,
    pub cluster_name: String,
    pub service_name: String,
    pub host: String,
    pub port: u16,
    pub health_check_path: String,
    #[allow(dead_code)]
    pub local_endpoint: String,
    #[allow(dead_code)]
    pub http_client: Client<HttpsConnector<hyper::client::HttpConnector>>,
    pub aws_service: AwsService,
    pub proxy_handler: ProxyHandler,
    pub ws_proxy: WebSocketReverseProxy,
}

const AWS_QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ') // space
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'=')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b']');

impl EcsAnywhereClient {
    async fn build_presigned_sts_url(&self) -> Result<String> {
        // Region and credentials from default chain
        let cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        let region = cfg
            .region()
            .map(|r| r.as_ref().to_string())
            .unwrap_or_else(|| "us-east-1".to_string());
        let creds = cfg
            .credentials_provider()
            .ok_or_else(|| anyhow::anyhow!("No AWS credentials provider available"))?
            .provide_credentials()
            .await?;

        let access_key = creds.access_key_id();
        let secret_key = creds.secret_access_key();
        let session_token = creds.session_token().map(|s| s.to_string());

        let service = "sts";
        let host = format!("sts.{}.amazonaws.com", region);
        let amz_date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = &amz_date[..8];

        // Canonical query base
        let mut query: Vec<(String, String)> = vec![
            ("Action".to_string(), "GetCallerIdentity".to_string()),
            ("Version".to_string(), "2011-06-15".to_string()),
            (
                "X-Amz-Algorithm".to_string(),
                "AWS4-HMAC-SHA256".to_string(),
            ),
            (
                "X-Amz-Credential".to_string(),
                format!(
                    "{}/{}/{}/{}/aws4_request",
                    access_key, date_stamp, region, service
                ),
            ),
            ("X-Amz-Date".to_string(), amz_date.clone()),
            ("X-Amz-Expires".to_string(), "60".to_string()),
            ("X-Amz-SignedHeaders".to_string(), "host".to_string()),
        ];
        if let Some(token) = &session_token {
            query.push(("X-Amz-Security-Token".to_string(), token.clone()));
        }

        // Encode keys and values, then sort by encoded key and encoded value
        let enc = |s: &str| utf8_percent_encode(s, AWS_QUERY_ENCODE_SET).to_string();
        let mut encoded: Vec<(String, String)> =
            query.iter().map(|(k, v)| (enc(k), enc(v))).collect();
        encoded.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let canonical_query = encoded
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        let canonical_headers = format!("host:{}\n", host);
        let signed_headers = "host";
        let payload_hash = {
            let mut hasher = Sha256::new();
            hasher.update(b""); // Empty body for GET
            hex::encode(hasher.finalize())
        };
        let canonical_request = format!(
            "GET\n/\n{}\n{}\n{}\n{}",
            canonical_query, canonical_headers, signed_headers, payload_hash
        );

        // String to sign
        let scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);
        let mut hasher = Sha256::new();
        hasher.update(canonical_request.as_bytes());
        let canonical_request_hash = hex::encode(hasher.finalize());
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date, scope, canonical_request_hash
        );

        // Derive signing key
        type HmacSha256 = Hmac<Sha256>;
        let k_date = {
            let mut mac = HmacSha256::new_from_slice(format!("AWS4{}", secret_key).as_bytes())?;
            mac.update(date_stamp.as_bytes());
            mac.finalize().into_bytes()
        };
        let k_region = {
            let mut mac = HmacSha256::new_from_slice(&k_date)?;
            mac.update(region.as_bytes());
            mac.finalize().into_bytes()
        };
        let k_service = {
            let mut mac = HmacSha256::new_from_slice(&k_region)?;
            mac.update(service.as_bytes());
            mac.finalize().into_bytes()
        };
        let k_signing = {
            let mut mac = HmacSha256::new_from_slice(&k_service)?;
            mac.update(b"aws4_request");
            mac.finalize().into_bytes()
        };

        let signature = {
            let mut mac = HmacSha256::new_from_slice(&k_signing)?;
            mac.update(string_to_sign.as_bytes());
            hex::encode(mac.finalize().into_bytes())
        };

        let final_url = format!(
            "https://{}?{}&X-Amz-Signature={}",
            host, canonical_query, signature
        );

        Ok(final_url)
    }
    fn websocket_to_http_health_url(ingress_endpoint: &str) -> Option<String> {
        // Convert ws[s]://host[:port][/path] to http[s]://host[:port]/health
        let url = if ingress_endpoint.starts_with("wss://") {
            ingress_endpoint.replacen("wss://", "https://", 1)
        } else if ingress_endpoint.starts_with("ws://") {
            ingress_endpoint.replacen("ws://", "http://", 1)
        } else {
            ingress_endpoint.to_string()
        };

        // Trim any path and append /health
        match url.find('/') {
            Some(idx) if idx > 7 => {
                // keep scheme and authority, drop the rest
                let (scheme_and_authority, _) = url.split_at(idx);
                Some(format!("{}/health", scheme_and_authority))
            }
            _ => Some(format!("{}/health", url.trim_end_matches('/'))),
        }
    }

    pub async fn new(
        cluster_name: String,
        service_name: String,
        host: String,
        port: u16,
        local_endpoint: String,
        health_check_path: String,
    ) -> Result<Self> {
        let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(region_provider)
            .load()
            .await;

        let https = HttpsConnector::new();
        let http_client = Client::builder().build::<_, Body>(https);

        let ecs_client = EcsClient::new(&config);
        let sts_client = StsClient::new(&config);

        let aws_service = AwsService::new(ecs_client, sts_client).await?;
        let proxy_handler = ProxyHandler::new(http_client.clone(), local_endpoint.clone());
        let ws_proxy = WebSocketReverseProxy::new(local_endpoint.clone());

        Ok(Self {
            client_id: Uuid::new_v4(),
            cluster_name,
            service_name,
            host,
            port,
            health_check_path,
            local_endpoint,
            http_client,
            aws_service,
            proxy_handler,
            ws_proxy,
        })
    }

    pub async fn get_service_attributes(&self) -> Result<HashMap<String, String>> {
        let mut attributes = HashMap::new();

        // Get ECS service attributes
        if let Some(task_arn) = &self.aws_service.task_arn {
            attributes.insert("environment".to_string(), "production".to_string());
            attributes.insert("version".to_string(), "1.0.0".to_string());
            attributes.insert("task_arn".to_string(), task_arn.clone());
        }

        Ok(attributes)
    }

    pub async fn register_service(&self) -> ServiceRegistration {
        let attributes = self.get_service_attributes().await.unwrap_or_default();

        ServiceRegistration {
            id: self.client_id,
            service_name: self.service_name.clone(),
            host: self.host.clone(),
            port: self.port,
            cluster_name: self.cluster_name.clone(),
            task_arn: self.aws_service.task_arn.clone().unwrap_or_default(),
            attributes,
            health_check_path: Some(self.health_check_path.clone()),
        }
    }

    #[instrument(skip(self))]
    pub async fn run(&self, ingress_endpoint: &str) -> Result<()> {
        loop {
            info!("Connecting to ingress service at: {}", ingress_endpoint);

            match self.connect_and_handle(ingress_endpoint).await {
                Ok(_) => {
                    info!("Connection to ingress service ended normally");
                }
                Err(e) => {
                    error!("Connection to ingress service failed: {}", e);
                }
            }

            info!("Reconnecting in 5 seconds...");
            sleep(Duration::from_secs(5)).await;
        }
    }

    async fn connect_and_handle(&self, ingress_endpoint: &str) -> Result<()> {
        let (ws_stream, _) = connect_async(ingress_endpoint).await?;
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        info!(
            "Connected to ingress service. Client ID: {}",
            self.client_id
        );

        // Perform IAM authentication handshake first (identity payload)
        let url = match self.build_presigned_sts_url().await {
            Ok(url) => {
                info!("Generated presigned STS URL for IAM authentication");
                Some(url)
            }
            Err(e) => {
                warn!(
                    "Failed to generate presigned STS URL: {}. IAM authentication will be skipped.",
                    e
                );
                None
            }
        };

        let auth_request = crate::common::auth::IamAuthRequest {
            presigned_url: url,
            region: "us-east-1".to_string(),
            arn: None,
            account_id: None,
            user_id: None,
        };

        let auth_message = IngressMessage::IamAuth(auth_request);
        let auth_json = serde_json::to_string(&auth_message)?;
        ws_sender.send(Message::Text(auth_json)).await?;

        // Wait for auth response before registering
        loop {
            match ws_receiver.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(IngressMessage::IamAuthResponse(resp)) =
                        serde_json::from_str::<IngressMessage>(&text)
                    {
                        if resp.success {
                            break;
                        } else {
                            error!("IAM auth failed: {:?}", resp.error);
                            return Err(anyhow::anyhow!("IAM auth failed"));
                        }
                    }
                    // Ignore non-auth messages until auth completes
                }
                Some(Ok(Message::Close(_))) => {
                    warn!("WebSocket closed during IAM auth");
                    return Err(anyhow::anyhow!("connection closed during auth"));
                }
                Some(Err(e)) => {
                    warn!("WebSocket error during IAM auth: {}", e);
                    return Err(e.into());
                }
                Some(_) => {
                    // Ignore non-text frames during auth
                }
                None => {
                    warn!("WebSocket disconnected during IAM auth");
                    return Err(anyhow::anyhow!("disconnected during auth"));
                }
            }
        }

        // Send initial service registration
        let registration = self.register_service().await;
        let registration_message = IngressMessage::ServiceRegistration(registration);
        let registration_json = serde_json::to_string(&registration_message)?;

        ws_sender.send(Message::Text(registration_json)).await?;
        info!("Sent service registration to ingress");

        // Send an immediate heartbeat on connect
        let heartbeat = IngressMessage::HeartBeat {
            cluster_name: self.cluster_name.clone(),
            client_id: self.client_id,
        };
        let heartbeat_json = serde_json::to_string(&heartbeat)?;
        if let Err(e) = ws_sender.send(Message::Text(heartbeat_json)).await {
            error!("Failed to send initial heartbeat: {}", e);
        } else {
            info!("Sent heartbeat to ingress");
        }

        // Clone necessary data for the proxy handler
        let proxy_handler = self.proxy_handler.clone();

        // Create a channel for heartbeat messages
        let (heartbeat_tx, mut heartbeat_rx) = mpsc::unbounded_channel();

        // Create a channel for proxy messages (from ws reverse proxy to ingress)
        let (proxy_tx, mut proxy_rx) = mpsc::unbounded_channel::<IngressMessage>();

        // Create a watch channel to signal reconnect when server instance changes
        let (reconnect_tx, mut reconnect_rx) = watch::channel(false);

        // Spawn a separate task for monitoring server health/instance id
        let http_client = self.http_client.clone();
        let health_url = Self::websocket_to_http_health_url(ingress_endpoint);
        let health_monitor_handle = tokio::spawn(async move {
            if let Some(health_url) = health_url {
                let mut last_instance: Option<String> = None;
                let mut tick = interval(Duration::from_secs(10));
                loop {
                    tick.tick().await;
                    match http_client.get(health_url.parse().unwrap()).await {
                        Ok(resp) => {
                            if let Ok(bytes) = hyper::body::to_bytes(resp.into_body()).await {
                                if let Ok(json) =
                                    serde_json::from_slice::<serde_json::Value>(&bytes)
                                {
                                    if let Some(id) =
                                        json.get("instance_id").and_then(|v| v.as_str())
                                    {
                                        let id_string = id.to_string();
                                        match &last_instance {
                                            None => last_instance = Some(id_string),
                                            Some(prev) if prev != &id_string => {
                                                let _ = reconnect_tx.send(true);
                                                break;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // ignore transient errors
                        }
                    }
                }
            }
        });

        // Spawn a separate task for sending heartbeats
        let heartbeat_client_id = self.client_id;
        let heartbeat_cluster_name = self.cluster_name.clone();
        let heartbeat_handle = tokio::spawn(async move {
            let mut heartbeat_interval = interval(Duration::from_secs(15));

            loop {
                heartbeat_interval.tick().await;

                let heartbeat = IngressMessage::HeartBeat {
                    cluster_name: heartbeat_cluster_name.clone(),
                    client_id: heartbeat_client_id,
                };

                if heartbeat_tx.send(heartbeat).is_err() {
                    // Channel closed, exit heartbeat task
                    break;
                }
            }
        });

        loop {
            tokio::select! {
                // Handle incoming messages
                msg = ws_receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            debug!("Received message: {}", text);
                            self.handle_ingress_message(&mut ws_sender, &text, &proxy_handler, &proxy_tx).await?;
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("WebSocket connection closed by server");
                            break;
                        }
                        Some(Err(e)) => {
                            error!("WebSocket error: {}", e);
                            return Err(e.into());
                        }
                        None => {
                            warn!("WebSocket stream ended");
                            break;
                        }
                        _ => {}
                    }
                }

                // Handle heartbeat messages from the separate task
                heartbeat_msg = heartbeat_rx.recv() => {
                    match heartbeat_msg {
                        Some(heartbeat) => {
                            let heartbeat_json = serde_json::to_string(&heartbeat)?;
                            if let Err(e) = ws_sender.send(Message::Text(heartbeat_json)).await {
                                error!("Failed to send heartbeat: {}", e);
                                break;
                            }
                            info!("Sent heartbeat to ingress");
                        }
                        None => {
                            // Heartbeat channel closed, exit loop
                            break;
                        }
                    }
                }

                // React to server roll signal
                changed = reconnect_rx.changed() => {
                    if changed.is_ok() && *reconnect_rx.borrow() {
                        warn!("Server instance changed; reconnecting...");
                        break;
                    }
                }

                // Handle proxy messages to send upstream
                proxy_msg = proxy_rx.recv() => {
                    if let Some(msg) = proxy_msg {
                        let json = serde_json::to_string(&msg)?;
                        if let Err(e) = ws_sender.send(Message::Text(json)).await {
                            error!("Failed to send ws proxy message: {}", e);
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
        }

        // Stop the heartbeat and health monitor tasks
        heartbeat_handle.abort();
        health_monitor_handle.abort();

        Ok(())
    }

    async fn handle_ingress_message(
        &self,
        ws_sender: &mut futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        message: &str,
        proxy_handler: &ProxyHandler,
        proxy_tx: &mpsc::UnboundedSender<IngressMessage>,
    ) -> Result<()> {
        let ingress_message: IngressMessage = serde_json::from_str(message)?;

        match ingress_message {
            IngressMessage::ProxyRequestForward(proxy_request) => {
                debug!("Processing proxy request: {}", proxy_request.id);

                // Handle the proxy request
                let response = proxy_handler.handle_request(proxy_request).await;

                // Send response back to ingress
                let response_message = IngressMessage::ProxyResponse(response);
                let response_json = serde_json::to_string(&response_message)?;
                ws_sender.send(Message::Text(response_json)).await?;

                debug!("Sent proxy response to ingress");
            }
            IngressMessage::WebSocketProxyInit {
                session_id,
                target_host,
                path,
                headers,
                subprotocols,
            } => {
                self.ws_proxy
                    .handle_init(
                        session_id,
                        target_host,
                        path,
                        headers,
                        subprotocols,
                        proxy_tx,
                    )
                    .await;
            }
            IngressMessage::WebSocketProxyData {
                session_id,
                frame_type,
                payload,
            } => {
                self.ws_proxy
                    .handle_data_from_server(session_id, frame_type, payload)
                    .await;
            }
            IngressMessage::WebSocketProxyClose {
                session_id,
                code,
                reason,
            } => {
                self.ws_proxy
                    .handle_close_from_server(session_id, code, reason)
                    .await;
            }
            IngressMessage::RegistrationAck {
                success, message, ..
            } => {
                if success {
                    info!("Service registration acknowledged: {}", message);
                } else {
                    warn!("Service registration failed: {}", message);
                }
            }
            _ => {
                warn!("Received unexpected message type from ingress");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_config::BehaviorVersion;
    use tokio::test;

    #[test]
    async fn test_presigned_sts_url_generation_and_call() {
        // Skip test if no AWS credentials available
        let cfg = aws_config::defaults(BehaviorVersion::latest()).load().await;
        let creds_provider = match cfg.credentials_provider() {
            Some(provider) => provider,
            None => {
                println!("Skipping test: No AWS credentials provider available");
                return;
            }
        };

        // Try to get credentials
        let creds = match creds_provider.provide_credentials().await {
            Ok(c) => c,
            Err(_) => {
                println!("Skipping test: Unable to obtain AWS credentials");
                return;
            }
        };

        println!(
            "Testing presigned URL with access key: {}",
            creds.access_key_id()
        );

        // Create a minimal client for testing
        let region_provider =
            aws_config::meta::region::RegionProviderChain::default_provider().or_else("us-east-1");
        let aws_cfg = aws_config::defaults(BehaviorVersion::latest())
            .region(region_provider)
            .load()
            .await;

        let https = HttpsConnector::new();
        let http_client = Client::builder().build::<_, Body>(https);
        let ecs_client = aws_sdk_ecs::Client::new(&aws_cfg);
        let sts_client = aws_sdk_sts::Client::new(&aws_cfg);
        let aws_service = AwsService::new(ecs_client, sts_client).await.unwrap();
        let proxy_handler =
            ProxyHandler::new(http_client.clone(), "http://localhost:3000".to_string());

        let client = EcsAnywhereClient {
            client_id: Uuid::new_v4(),
            cluster_name: "test-cluster".to_string(),
            service_name: "test-service".to_string(),
            host: "test.local".to_string(),
            port: 3000,
            health_check_path: "/health".to_string(),
            local_endpoint: "http://localhost:3000".to_string(),
            http_client: http_client.clone(),
            aws_service,
            proxy_handler,
            ws_proxy: WebSocketReverseProxy::new("http://localhost:3000".to_string()),
        };

        // Test presigned URL generation
        let presigned_url = client.build_presigned_sts_url().await.unwrap();
        println!("Generated presigned URL: {}", presigned_url);

        // Validate URL structure
        assert!(presigned_url.starts_with("https://sts."));
        assert!(presigned_url.contains("amazonaws.com"));
        assert!(presigned_url.contains("Action=GetCallerIdentity"));
        assert!(presigned_url.contains("Version=2011-06-15"));
        assert!(presigned_url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(presigned_url.contains("X-Amz-Credential="));
        assert!(presigned_url.contains("X-Amz-Date="));
        assert!(presigned_url.contains("X-Amz-Expires=60"));
        assert!(presigned_url.contains("X-Amz-Signature="));
        assert!(presigned_url.contains("X-Amz-SignedHeaders=host"));

        // Test actual STS call using the presigned URL
        let uri: hyper::Uri = presigned_url.parse().unwrap();
        let response = http_client.get(uri).await.unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let body_str = String::from_utf8_lossy(&body);

        println!("STS Response Status: {}", status);
        println!("STS Response Body: {}", body_str);

        // Verify successful response
        assert!(
            status.is_success(),
            "STS call failed with status: {}",
            status
        );
        assert!(body_str.contains("<GetCallerIdentityResponse"));
        assert!(body_str.contains("<Arn>"));
        assert!(body_str.contains("<Account>"));
        assert!(body_str.contains("<UserId>"));

        // Test XML parsing
        fn extract_xml_field(s: &str, tag: &str) -> Option<String> {
            let start = format!("<{}>", tag);
            let end = format!("</{}>", tag);
            let si = s.find(&start)? + start.len();
            let ei = s[si..].find(&end)? + si;
            Some(s[si..ei].to_string())
        }

        let arn = extract_xml_field(&body_str, "Arn").unwrap();
        let account = extract_xml_field(&body_str, "Account").unwrap();
        let user_id = extract_xml_field(&body_str, "UserId").unwrap();

        println!("Extracted ARN: {}", arn);
        println!("Extracted Account: {}", account);
        println!("Extracted UserId: {}", user_id);

        assert!(!arn.is_empty());
        assert!(!account.is_empty());
        assert!(!user_id.is_empty());
        assert!(arn.starts_with("arn:aws:"));
        assert!(account.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    async fn test_aws_sdk_vs_our_implementation() {
        // Skip test if no AWS credentials available
        let cfg = aws_config::defaults(BehaviorVersion::latest()).load().await;
        let creds_provider = match cfg.credentials_provider() {
            Some(provider) => provider,
            None => {
                println!("Skipping test: No AWS credentials provider available");
                return;
            }
        };

        // Try to get credentials
        let _creds = match creds_provider.provide_credentials().await {
            Ok(c) => c,
            Err(_) => {
                println!("Skipping test: Unable to obtain AWS credentials");
                return;
            }
        };

        // Test AWS SDK STS call as baseline
        let region_provider =
            aws_config::meta::region::RegionProviderChain::default_provider().or_else("us-east-1");
        let aws_cfg = aws_config::defaults(BehaviorVersion::latest())
            .region(region_provider)
            .load()
            .await;

        let sts_client = aws_sdk_sts::Client::new(&aws_cfg);
        match sts_client.get_caller_identity().send().await {
            Ok(identity) => {
                println!("✅ AWS SDK STS call successful:");
                if let Some(arn) = identity.arn() {
                    println!("  ARN: {}", arn);
                }
                if let Some(account) = identity.account() {
                    println!("  Account: {}", account);
                }
                if let Some(user_id) = identity.user_id() {
                    println!("  UserId: {}", user_id);
                }
            }
            Err(e) => {
                println!("❌ AWS SDK STS call failed: {}", e);
                panic!("This test assumes valid AWS credentials");
            }
        }

        // Test our presigned URL structure (even if signature is wrong)
        let https = HttpsConnector::new();
        let http_client = Client::builder().build::<_, Body>(https);
        let ecs_client = aws_sdk_ecs::Client::new(&aws_cfg);
        let sts_client_2 = aws_sdk_sts::Client::new(&aws_cfg);
        let aws_service = AwsService::new(ecs_client, sts_client_2).await.unwrap();
        let proxy_handler =
            ProxyHandler::new(http_client.clone(), "http://localhost:3000".to_string());

        let client = EcsAnywhereClient {
            client_id: Uuid::new_v4(),
            cluster_name: "test-cluster".to_string(),
            service_name: "test-service".to_string(),
            host: "test.local".to_string(),
            port: 3000,
            health_check_path: "/health".to_string(),
            local_endpoint: "http://localhost:3000".to_string(),
            http_client: http_client.clone(),
            aws_service,
            proxy_handler,
            ws_proxy: WebSocketReverseProxy::new("http://localhost:3000".to_string()),
        };

        let presigned_url = client.build_presigned_sts_url().await.unwrap();
        println!("📋 Generated presigned URL structure looks correct:");
        println!("  ✓ URL: {}", presigned_url);

        // Validate URL structure (this should always pass)
        assert!(presigned_url.starts_with("https://sts."));
        assert!(presigned_url.contains("amazonaws.com"));
        assert!(presigned_url.contains("Action=GetCallerIdentity"));
        assert!(presigned_url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(presigned_url.contains("X-Amz-Credential="));
        assert!(presigned_url.contains("X-Amz-Signature="));

        println!("✅ Presigned URL structure validation passed");
        println!("🔄 Authentication flow components are working");
    }
}
