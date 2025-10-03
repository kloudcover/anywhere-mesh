# Application Load Balancer & Route 53 Routing

The infrastructure stack provisions a public-facing Application Load Balancer (ALB) that fronts the mesh ingress service. HTTPS listeners expose both HTTP proxy traffic (443 -> container port 8080) and WebSocket upgrades (8082 -> container port 8082). A wildcard ACM certificate for `*.e2e.clustermaestro.com` is attached so any subdomain under that hosted zone can be routed through the ALB.

## Deploying the ALB

Deploy using the Taskfile helper or CDK commands:

```bash
# Taskfile shortcut
task deploy-e2e-infra

# Or the underlying CDK flow
cd infra
npm install
npm run build
cdk deploy --require-approval never
```

The stack creates:
- An internet-facing ALB with security groups that allow 443 and 8082.
- Two target groups with `/health` checks on ports 8080 and 8082.
- HTTPS listeners on 443 (default fixed 503 response until the mesh service is healthy) and 8082 with host-based routing for `*.e2e.clustermaestro.com`.
- A wildcard `A` record in the Route 53 hosted zone pointing to the ALB (created as `*` inside `e2e.clustermaestro.com`).

## Retrieving the ALB DNS name

```bash
aws elbv2 describe-load-balancers \
  --names EcsAnywhereMesh-e2eMeshAlb \
  --query 'LoadBalancers[0].DNSName' \
  --output text
```

You can also read the CloudFormation output `AlbDnsName` (visible in the CDK deploy output or via `aws cloudformation describe-stacks`).

## Creating additional Route 53 records

The wildcard record routes any subdomain to the ALB. To add an explicit record (for example, `mesh.e2e.clustermaestro.com`), use an alias `A` record targeting the same load balancer:

```bash
cat <<'JSON' > change-batch.json
{
  "Comment": "Alias mesh.e2e.clustermaestro.com to the mesh ALB",
  "Changes": [
    {
      "Action": "UPSERT",
      "ResourceRecordSet": {
        "Name": "mesh.e2e.clustermaestro.com",
        "Type": "A",
        "AliasTarget": {
          "HostedZoneId": "<ALB_HOSTED_ZONE_ID>",
          "DNSName": "<ALB_DNS_NAME>",
          "EvaluateTargetHealth": false
        }
      }
    }
  ]
}
JSON

aws route53 change-resource-record-sets \
  --hosted-zone-id ZU20SE53GVCYV \
  --change-batch file://change-batch.json
```

Replace `<ALB_DNS_NAME>` with the DNS name from the earlier step and `<ALB_HOSTED_ZONE_ID>` with the canonical hosted zone ID (`aws elbv2 describe-load-balancers --query 'LoadBalancers[0].CanonicalHostedZoneId'`).

## Testing routing

1. Verify the ingress service is healthy:
   ```bash
   curl https://<ALB_DNS_NAME>/health --resolve 'mesh.e2e.clustermaestro.com:443:<ALB_IP>'
   ```
   Or rely on DNS once propagation finishes:
   ```bash
   curl -H 'Host: mesh.e2e.clustermaestro.com' https://<ALB_DNS_NAME>/health
   ```

2. Route WebSocket traffic (the listener on 8082 enforces host headers under the wildcard domain):
   ```bash
   wscat -c wss://<ALB_DNS_NAME>:8082/ws -H 'Host: mesh.e2e.clustermaestro.com'
   ```

3. Load-test via the provided k6 scripts:
   ```bash
   TARGET_HOST=mesh.e2e.clustermaestro.com \
   BASE_URL=https://<ALB_DNS_NAME> \
   WS_URL=wss://<ALB_DNS_NAME>:8082 \
   k6 run load-test/http-smoke.js
   ```

## Troubleshooting

- **ALB returns 503** - ensure the mesh Fargate task is passing the `/health` target group check and that security groups allow traffic on ports 8080/8082.
- **`SSL_ERROR_BAD_CERT_DOMAIN`** - the certificate only covers subdomains of `e2e.clustermaestro.com`; access via the raw DNS name or add a matching alias.
- **WebSocket 403** - confirm your request uses a host header that matches the wildcard condition (`*.e2e.clustermaestro.com`).
