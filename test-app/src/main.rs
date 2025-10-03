use axum::{
    extract::{ws::{WebSocket, WebSocketUpgrade}, Path, Query},
    http::{HeaderMap, Method, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use hostname;
use serde::Serialize;
use serde_json::json;
use std::{
    collections::HashMap,
    env,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use axum::extract::ws::Message;
use futures_util::{sink::SinkExt, stream::StreamExt};
use tokio::time::sleep;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};

// Service metadata
const SERVICE_NAME: &str = "test-app";
const SERVICE_VERSION: &str = "1.0.0";
const SERVICE_PORT: u16 = 3000;

#[derive(Clone)]
struct AppState {
    start_time: Instant,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    service: String,
    timestamp: DateTime<Utc>,
    uptime_seconds: f64,
}

#[derive(Serialize)]
struct ServiceInfo {
    service: String,
    version: String,
    port: u16,
    hostname: String,
    timestamp: DateTime<Utc>,
    uptime: f64,
    environment: HashMap<String, String>,
}

#[derive(Serialize)]
struct EchoResponse {
    method: String,
    url: String,
    args: HashMap<String, String>,
    headers: HashMap<String, String>,
    timestamp: DateTime<Utc>,
    service: String,
}

#[derive(Serialize)]
struct TimeResponse {
    timestamp: DateTime<Utc>,
    unix_timestamp: f64,
    formatted: String,
    service: String,
}

#[derive(Serialize)]
struct HeadersResponse {
    headers: HashMap<String, String>,
    method: String,
    path: String,
    query_string: String,
    remote_addr: Option<String>,
    service: String,
    timestamp: DateTime<Utc>,
}

#[derive(Serialize)]
struct EnvironmentResponse {
    environment: HashMap<String, String>,
    service: String,
    timestamp: DateTime<Utc>,
}

#[derive(Serialize)]
struct StressResponse {
    requested_delay: u64,
    actual_delay: f64,
    service: String,
    timestamp: DateTime<Utc>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    message: String,
    service: String,
    available_endpoints: Vec<String>,
    timestamp: DateTime<Utc>,
}

async fn get_hostname() -> String {
    hostname::get()
        .unwrap_or_else(|_| "unknown".into())
        .to_string_lossy()
        .to_string()
}

fn get_uptime(start_time: Instant) -> f64 {
    start_time.elapsed().as_secs_f64()
}

fn get_current_timestamp() -> DateTime<Utc> {
    Utc::now()
}

fn get_unix_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn headers_to_hashmap(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect()
}

fn get_filtered_environment() -> HashMap<String, String> {
    let sensitive_keywords = ["PASSWORD", "SECRET", "KEY", "TOKEN", "CREDENTIAL"];

    env::vars()
        .filter(|(k, _)| {
            !sensitive_keywords
                .iter()
                .any(|sensitive| k.to_uppercase().contains(sensitive))
        })
        .collect()
}

// Route handlers
async fn index() -> impl IntoResponse {
    match tokio::fs::read_to_string("index.html").await {
        Ok(content) => Html(content),
        Err(_) => Html("<h1>Test App</h1><p>Static file not found</p>".to_string()),
    }
}

async fn health_check(state: axum::extract::State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        service: SERVICE_NAME.to_string(),
        timestamp: get_current_timestamp(),
        uptime_seconds: get_uptime(state.start_time),
    })
}

async fn service_info(state: axum::extract::State<Arc<AppState>>) -> Json<ServiceInfo> {
    let hostname = get_hostname().await;

    Json(ServiceInfo {
        service: SERVICE_NAME.to_string(),
        version: SERVICE_VERSION.to_string(),
        port: SERVICE_PORT,
        hostname,
        timestamp: get_current_timestamp(),
        uptime: get_uptime(state.start_time),
        environment: get_filtered_environment(),
    })
}

async fn echo(
    method: Method,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Json<EchoResponse> {
    let url = format!("http://{}{}", get_hostname().await, "unknown");
    let headers_map = headers_to_hashmap(&headers);

    Json(EchoResponse {
        method: method.to_string(),
        url,
        args: params,
        headers: headers_map,
        timestamp: get_current_timestamp(),
        service: SERVICE_NAME.to_string(),
    })
}

async fn current_time() -> Json<TimeResponse> {
    let now = get_current_timestamp();
    let formatted = now.format("%Y-%m-%d %H:%M:%S UTC").to_string();

    Json(TimeResponse {
        timestamp: now,
        unix_timestamp: get_unix_timestamp(),
        formatted,
        service: SERVICE_NAME.to_string(),
    })
}

async fn request_headers(
    method: Method,
    headers: HeaderMap,
) -> Json<HeadersResponse> {
    Json(HeadersResponse {
        headers: headers_to_hashmap(&headers),
        method: method.to_string(),
        path: "unknown".to_string(), // Would need middleware to extract this
        query_string: "".to_string(),
        remote_addr: None, // Would need middleware to extract this
        service: SERVICE_NAME.to_string(),
        timestamp: get_current_timestamp(),
    })
}

async fn environment() -> Json<EnvironmentResponse> {
    Json(EnvironmentResponse {
        environment: get_filtered_environment(),
        service: SERVICE_NAME.to_string(),
        timestamp: get_current_timestamp(),
    })
}

async fn stress_test(
    Path(delay): Path<u64>,
) -> Result<Json<StressResponse>, (StatusCode, Json<serde_json::Value>)> {
    if delay > 30 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Delay too long, max 30 seconds"
            })),
        ));
    }

    let start = Instant::now();
    sleep(Duration::from_secs(delay)).await;
    let actual_delay = start.elapsed().as_secs_f64();

    Ok(Json(StressResponse {
        requested_delay: delay,
        actual_delay,
        service: SERVICE_NAME.to_string(),
        timestamp: get_current_timestamp(),
    }))
}

// WebSocket handlers
async fn websocket_handler(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_websocket)
}

async fn handle_websocket(socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    
    println!("🔌 WebSocket connection established");
    
    // Send welcome message
    let welcome_msg = json!({
        "type": "welcome",
        "service": SERVICE_NAME,
        "timestamp": get_current_timestamp(),
        "message": "WebSocket connection established"
    });
    
    if sender.send(Message::Text(welcome_msg.to_string())).await.is_err() {
        println!("❌ Failed to send welcome message");
        return;
    }
    
    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                println!("📨 Received: {}", text);
                
                // Echo back with metadata
                let response = json!({
                    "type": "echo",
                    "original_message": text,
                    "service": SERVICE_NAME,
                    "timestamp": get_current_timestamp(),
                    "connection_info": "WebSocket echo response"
                });
                
                if sender.send(Message::Text(response.to_string())).await.is_err() {
                    println!("❌ Failed to send echo response");
                    break;
                }
            }
            Ok(Message::Binary(data)) => {
                println!("📨 Received binary data: {} bytes", data.len());
                // Echo back binary data
                if sender.send(Message::Binary(data)).await.is_err() {
                    println!("❌ Failed to send binary echo");
                    break;
                }
            }
            Ok(Message::Ping(data)) => {
                println!("🏓 Received ping");
                if sender.send(Message::Pong(data)).await.is_err() {
                    println!("❌ Failed to send pong");
                    break;
                }
            }
            Ok(Message::Pong(_)) => {
                println!("🏓 Received pong");
            }
            Ok(Message::Close(close_frame)) => {
                println!("🔌 WebSocket closing: {:?}", close_frame);
                break;
            }
            Err(e) => {
                println!("❌ WebSocket error: {}", e);
                break;
            }
        }
    }
    
    println!("🔌 WebSocket connection closed");
}

async fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "Not Found".to_string(),
            message: "The requested URL was not found".to_string(),
            service: SERVICE_NAME.to_string(),
            available_endpoints: vec![
                "/".to_string(),
                "/api/health".to_string(),
                "/api/info".to_string(),
                "/api/echo".to_string(),
                "/api/time".to_string(),
                "/api/headers".to_string(),
                "/api/environment".to_string(),
                "/api/stress/<delay>".to_string(),
                "/ws".to_string(),
                "/api/ws".to_string(),
            ],
            timestamp: get_current_timestamp(),
        }),
    )
}

#[tokio::main]
async fn main() {
    let start_time = Instant::now();
    let state = Arc::new(AppState { start_time });

    println!("🚀 Starting {} v{}", SERVICE_NAME, SERVICE_VERSION);
    println!("📡 Port: {}", SERVICE_PORT);
    println!("🏠 Hostname: {}", get_hostname().await);
    println!("⏰ Started at: {}", get_current_timestamp().to_rfc3339());
    println!("{}", "=".repeat(50));

    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(Any);

    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(health_check))
        .route("/api/health", get(health_check))
        .route("/api/info", get(service_info))
        .route("/api/echo", get(echo))
        .route("/api/time", get(current_time))
        .route("/api/headers", get(request_headers))
        .route("/api/environment", get(environment))
        .route("/api/stress/:delay", get(stress_test))
        .route("/ws", get(websocket_handler))
        .route("/api/ws", get(websocket_handler))
        .fallback(not_found)
        .layer(ServiceBuilder::new().layer(cors))
        .with_state(state);

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| SERVICE_PORT.to_string())
        .parse::<u16>()
        .expect("PORT must be a valid number");

    let addr = format!("0.0.0.0:{}", port);
    println!("🌐 Attempting to bind to {}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => {
            println!("✅ Successfully bound to {}", addr);
            listener
        }
        Err(e) => {
            eprintln!("❌ Failed to bind to {}: {}", addr, e);
            eprintln!("💡 Try using a different port:");
            eprintln!("   cargo run");
            eprintln!("   PORT=3001 cargo run");
            eprintln!("   docker run -p 3001:3001 -e PORT=3001 test-app-rust");
            std::process::exit(1);
        }
    };

    println!("🚀 Server started successfully!");
    println!("🌐 Listening on http://{}", addr);
    println!("📊 Health check: http://{}/api/health", addr);
    println!("🔌 WebSocket: ws://{}/ws", addr);
    println!("🔌 WebSocket API: ws://{}/api/ws", addr);

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
