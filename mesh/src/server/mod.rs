use anyhow::Result;
use hyper::service::{make_service_fn, service_fn};
use hyper::Server;
use std::convert::Infallible;
use std::net::SocketAddr;

use crate::commands::ServerCommand;
use service::CombinedIngressService;
use tracing::{error, info};
// Import handlers to bring the impl blocks into scope
#[allow(unused_imports)]
use handlers::{alb, health, websocket};

pub mod auth;
mod dispatcher;
mod error;
mod handlers;
mod registry;
mod router;
mod service;
mod ws_proxy;

pub async fn run(args: ServerCommand) -> Result<()> {
    println!("Starting Anywhere Mesh Server...");

    info!("🚀 Starting Combined Ingress Service");
    info!("📡 ALB port: {}", args.alb_port);
    info!("🔌 WebSocket port: {}", args.websocket_port);
    info!("⏱️  Request timeout: {}s", args.request_timeout);

    let service = CombinedIngressService::new();

    // Start ALB HTTP server
    let alb_service = service.clone();
    let alb_port = args.alb_port;
    let alb_handle = tokio::spawn(async move {
        let make_svc = make_service_fn(move |_conn| {
            let service = alb_service.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let service = service.clone();
                    async move { service.handle_alb_request(req).await }
                }))
            }
        });

        let alb_addr = SocketAddr::from(([0, 0, 0, 0], alb_port));
        let server = Server::bind(&alb_addr).serve(make_svc);

        info!("🌐 ALB HTTP server listening on {}", alb_addr);

        if let Err(e) = server.await {
            error!("ALB HTTP server error: {}", e);
        }
    });

    // Start internal HTTP server (for health checks, metrics)
    let internal_service = service.clone();
    let internal_handle = tokio::spawn(async move {
        let make_svc = make_service_fn(move |_conn| {
            let service = internal_service.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let service = service.clone();
                    async move { service.handle_internal_request(req).await }
                }))
            }
        });

        let internal_addr = SocketAddr::from(([0, 0, 0, 0], 8081));
        let server = Server::bind(&internal_addr).serve(make_svc);

        info!("🏥 Internal HTTP server listening on {}", internal_addr);

        if let Err(e) = server.await {
            error!("Internal HTTP server error: {}", e);
        }
    });

    // Start WebSocket/Health Check server
    let ws_service = service.clone();
    let websocket_port = args.websocket_port;
    let ws_handle = tokio::spawn(async move {
        let make_svc = make_service_fn(move |_conn| {
            let service = ws_service.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let service = service.clone();
                    async move { service.handle_websocket_or_health_request(req).await }
                }))
            }
        });

        let ws_addr = SocketAddr::from(([0, 0, 0, 0], websocket_port));
        let server = Server::bind(&ws_addr).serve(make_svc);

        info!("🔌 WebSocket/Health server listening on {}", ws_addr);

        if let Err(e) = server.await {
            error!("WebSocket/Health server error: {}", e);
        }
    });

    println!("✅ All servers started successfully!");

    println!();
    println!("🎯 Server Endpoints:");
    println!("   🌐 ALB Traffic:     http://0.0.0.0:{}", args.alb_port);
    println!("   🏥 Health/Metrics:  http://0.0.0.0:8081");
    println!(
        "   🔌 WebSocket:       ws://0.0.0.0:{}",
        args.websocket_port
    );
    println!();
    println!("📊 Ready to accept connections!");
    println!();

    // Wait for all servers - if any exit, we should exit with an error
    tokio::select! {
        result = alb_handle => {
            error!("ALB server exited unexpectedly: {:?}", result);
            std::process::exit(1);
        },
        result = internal_handle => {
            error!("Internal server exited unexpectedly: {:?}", result);
            std::process::exit(1);
        },
        result = ws_handle => {
            error!("WebSocket server exited unexpectedly: {:?}", result);
            std::process::exit(1);
        },
    }
}
