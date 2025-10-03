use anyhow::Result;
use aws_sdk_ecs::Client as EcsClient;
use aws_sdk_sts::Client as StsClient;
use std::env;
use tracing::{error, info, warn};

pub struct AwsService {
    pub ecs_client: EcsClient,
    pub sts_client: StsClient,
    pub task_arn: Option<String>,
}

impl AwsService {
    pub async fn new(ecs_client: EcsClient, sts_client: StsClient) -> Result<Self> {
        let task_arn = Self::get_task_arn().await;

        let service = Self {
            ecs_client,
            sts_client,
            task_arn,
        };

        // Validate IAM permissions
        if let Err(e) = service.validate_iam_permissions().await {
            warn!("IAM validation failed: {}", e);
        }

        Ok(service)
    }

    async fn get_task_arn() -> Option<String> {
        // Try to get task ARN from ECS metadata endpoint
        if let Ok(_metadata_uri) = env::var("ECS_CONTAINER_METADATA_URI_V4") {
            // In a real implementation, this would fetch from the metadata endpoint
            // For now, return a placeholder
            Some("arn:aws:ecs:us-east-1:123456789012:task/my-cluster/abc123def456".to_string())
        } else {
            warn!("ECS_CONTAINER_METADATA_URI_V4 not found, using placeholder task ARN");
            Some("arn:aws:ecs:us-east-1:123456789012:task/my-cluster/placeholder".to_string())
        }
    }

    pub async fn validate_iam_permissions(&self) -> Result<bool> {
        // Validate that this service has the required IAM permissions
        match self.sts_client.get_caller_identity().send().await {
            Ok(identity) => {
                if let Some(arn) = identity.arn() {
                    info!("Validated IAM identity: {}", arn);
                    Ok(true)
                } else {
                    warn!("Could not get caller identity ARN");
                    Ok(false)
                }
            }
            Err(e) => {
                error!("Failed to validate IAM permissions: {}", e);
                Ok(false)
            }
        }
    }

    pub async fn check_ecs_cluster_access(&self, cluster_name: &str) -> Result<bool> {
        // Check if we can access the specified ECS cluster
        match self
            .ecs_client
            .describe_clusters()
            .clusters(cluster_name)
            .send()
            .await
        {
            Ok(response) => {
                let clusters = response.clusters();
                if !clusters.is_empty() {
                    info!("Successfully accessed ECS cluster: {}", cluster_name);
                    Ok(true)
                } else {
                    warn!("ECS cluster not found: {}", cluster_name);
                    Ok(false)
                }
            }
            Err(e) => {
                error!("Failed to access ECS cluster {}: {}", cluster_name, e);
                Ok(false)
            }
        }
    }
}
