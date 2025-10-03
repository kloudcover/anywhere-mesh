# Anywhere Mesh Infrastructure (CDK)

This directory contains the AWS CDK (TypeScript) stack that provisions the end-to-end environment for Anywhere Mesh: an internet-facing ALB (HTTPS 443 and 8082), ECS Cluster and Fargate services, Route53 records, IAM, CloudWatch logs, and Service Connect namespace.

## Prerequisites

- Node.js 18+ and npm
- AWS credentials configured (aws-vault or SSO recommended)
- An existing VPC and public subnet
- An existing Route53 hosted zone for your domain

## Install & Build

```bash
cd infra
npm install
npm run build
```

## Configure `params.json`

Copy and edit `params.json` to match your environment. Required fields:

```json
{
  "account": "123456789012",
  "region": "us-west-2",
  "hostedZoneId": "ZXXXXXXXXXXXXX",
  "zoneName": "example.com",
  "domainName": "example.com",
  "vpcId": "vpc-xxxxxxxx",
  "subnetId": "subnet-xxxxxxxx",
  "clusterName": "e2e-mesh-cluster",
  "allowedRoleArns": "arn:aws:sts::123456789012:assumed-role/*",
  "serviceConnectNamespace": "mesh-e2e.local"
}
```

Notes:

- `domainName` is used for the ALB certificate and `*.domainName` SAN.
- A wildcard `A` record is created in the hosted zone pointing to the ALB.
- Listeners: HTTPS 443 for HTTP routing, HTTPS 8082 for WebSocket registration.

## Synthesize, Bootstrap, Deploy

```bash
cdk synth
cdk bootstrap   # once per account/region
cdk deploy --require-approval never
```

On success, the stack outputs include:

- `AlbDnsName`: ALB DNS name
- `ClusterName`: ECS cluster name
- `ServiceName`: Mesh service name
- `TestServiceName`: Service Connect test client
- `ServiceConnectNamespace`: Cloud Map namespace

## Entrypoint

The CDK app reads `params.json` in `bin/infra.ts` and constructs the `InfraStack` defined in `lib/stack.ts`.

```ts
// bin/infra.ts (excerpt)
new InfraStack(app, "ecs-anywhere-mesh-e2e", {
  env: { account: params.account, region: params.region },
  hostedZoneId: params.hostedZoneId,
  zoneName: params.zoneName,
  domainName: params.domainName,
  vpcId: params.vpcId,
  subnetId: params.subnetId,
  clusterName: params.clusterName,
  allowedRoleArns: params.allowedRoleArns,
  serviceConnectNamespace: params.serviceConnectNamespace,
});
```

## Next Steps

- For end-to-end validation and convenience workflows, see the repository root `Taskfile.yml` (e.g., `task deploy-e2e-infra`).
- For load testing guidance, see `../load-test/README.md`.
