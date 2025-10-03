# Service Connect Integration

The end-to-end (e2e) CDK stack enables Amazon ECS Service Connect so mesh-aware workloads can resolve and call the ingress service without managing private networking or manual Cloud Map registrations. Service Connect runs inside the cluster namespace `mesh-e2e.local` and publishes the ingress service as `mesh-ingress.mesh-e2e.local`.

## Deploying the Service Connect resources

1. Synthesize and deploy the stack (installs the Service Connect namespace, Fargate services, listeners, and log groups):
   ```bash
   cd infra
   npm install
   npm run build
   cdk deploy --require-approval never
   ```
   Or use the Taskfile shortcut: `task deploy-e2e-infra`.
2. The stack adds the default Service Connect namespace when creating the ECS cluster and configures the ingress Fargate task definition with the `mesh-ingress` port mapping. During deployment the service automatically registers with AWS Cloud Map.

## Verifying DNS and connectivity

### From your workstation (execute-command)

The Taskfile ships a helper (`task exec-test-app`) that opens a shell in the Service Connect test client. You can also run the commands manually:

```bash
SERVICE_ARN=$(aws ecs list-services \
  --cluster e2e-mesh-cluster \
  --query 'serviceArns[?contains(@, `ecs-anywhere-mesh-e2e-TestService`)]' \
  --output text | head -n1)

TASK_ARN=$(aws ecs list-tasks \
  --cluster e2e-mesh-cluster \
  --service-name "${SERVICE_ARN##*/}" \
  --query 'taskArns[0]' \
  --output text)

aws ecs execute-command \
  --cluster e2e-mesh-cluster \
  --task "$TASK_ARN" \
  --container test-client \
  --interactive \
  --command "/bin/sh"
```

Once connected, confirm DNS and health:

```bash
nslookup mesh-ingress.mesh-e2e.local
curl -v http://mesh-ingress.mesh-e2e.local:8080/health
```

Both commands should succeed if the Service Connect namespace and discovery are configured correctly.

### CloudWatch logs

The CDK stack enables two log groups to help with troubleshooting:

- `/ecs/<stack-name>/mesh-server` captures ingress service logs (ports 8080/8082).
- `/ecs/<stack-name>/service-connect-test` captures the continuous connectivity probe from the Service Connect test client.

Use these logs to identify registration failures, DNS timeouts, or health-check errors.

## Adding new Service Connect consumers

1. Ensure the ECS service uses the same cluster (`e2e-mesh-cluster`) and references the `mesh-e2e.local` namespace in its `serviceConnectConfiguration`.
2. Add the ingress service as a dependency by resolving `mesh-ingress.mesh-e2e.local` and calling port 8080 (HTTP). The Service Connect proxy injects TLS-free traffic within the namespace; use HTTPS only when crossing the ALB.
3. Grant the task role permissions for `servicediscovery:*Instance*` actions if the service will also register endpoints.

## Common issues

- **DNS resolution fails (`SERVFAIL` or `NXDOMAIN`)** - verify the Fargate task runs in the cluster where Service Connect is enabled and that the namespace (`mesh-e2e.local`) exists.
- **HTTP requests hang** - confirm the ingress service is passing health checks (`/health`) so Service Connect keeps the endpoint in rotation.
- **`aws ecs execute-command` errors** - make sure `aws-cli` v2 and Session Manager plugin are installed locally, and the IAM principal has `ecs:ExecuteCommand`/`ssmmessages:*` permissions.
