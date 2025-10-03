use anyhow::Result;
use std::env;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

use crate::commands::ClientCommand;
use client_impl::EcsAnywhereClient;

mod aws;
mod client_impl;
mod proxy;

pub async fn run(args: ClientCommand) -> Result<()> {
    println!("Starting  Mesh Client...");

    info!("🚀 Starting Anywhere Mesh Client");
    info!("📡 Ingress endpoint: {}", args.ingress_endpoint);

    // Check for IAM validation skip
    if args.skip_iam_validation || env::var("SKIP_IAM_VALIDATION").is_ok() {
        warn!("⚠️  IAM validation is disabled - this should only be used in development!");
    }

    // Fall back to single-service CLI mode
    info!("📦 Service: {}", args.service_name);
    info!("🏢 Cluster: {}", args.cluster_name);
    info!("🌐 Host: {}", args.host);
    info!("🔌 Port: {}", args.port);
    info!("🎯 Local endpoint: {}", args.local_endpoint);
    info!("📡 Ingress endpoint: {}", args.ingress_endpoint);

    // Create the client
    let client = EcsAnywhereClient::new(
        args.cluster_name.clone(),
        args.service_name.clone(),
        args.host.clone(),
        args.port,
        args.local_endpoint.clone(),
        args.health_check_path.clone(),
    )
    .await?;

    // Validate ECS cluster access (unless skipped)
    if !args.skip_iam_validation {
        println!("Validating ECS cluster access...");
        if let Err(e) = client
            .aws_service
            .check_ecs_cluster_access(&client.cluster_name)
            .await
        {
            println!("❌ Failed to validate ECS cluster access");
            error!("Failed to validate ECS cluster access: {}", e);
            return Err(e);
        }
    }

    // Start health check monitoring
    let health_client = client.proxy_handler.clone();
    let health_path = args.health_check_path.clone();
    tokio::spawn(async move {
        let mut health_interval = interval(Duration::from_secs(30));

        loop {
            health_interval.tick().await;

            match health_client.health_check(&health_path).await {
                Ok(true) => {
                    // Service is healthy
                }
                Ok(false) => {
                    warn!("⚠️  Local service health check failed");
                }
                Err(e) => {
                    error!("❌ Health check error: {}", e);
                }
            }
        }
    });

    println!("✅ Client initialized successfully!");

    println!();
    println!("🎯 Client Configuration:");
    println!("   📦 Service:         {}", args.service_name);
    println!("   🏢 Cluster:         {}", args.cluster_name);
    println!("   🌐 Routing Host:    {}", args.host);
    println!("   🔌 Service Port:    {}", args.port);
    println!("   🎯 Local Service:   {}", args.local_endpoint);
    println!("   📡 Ingress:         {}", args.ingress_endpoint);
    println!();
    println!("🔗 Connecting to ingress service...");
    println!();

    // Start the main client loop
    client.run(&args.ingress_endpoint).await?;

    Ok(())
}
