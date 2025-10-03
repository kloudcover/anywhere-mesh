## Security Hardening Plan

This document tracks concrete security improvements to harden the ECS Anywhere Mesh service for open-source use. Items are grouped by priority with clear, actionable steps and recommended defaults.

### Scope

- Ingress server (`mesh/src/server/*`)
- Client (`mesh/src/client/*`)
- Infra/CDK (`infra/lib/infra-stack.ts`)
- Configuration (`config/*.yaml`, env vars)

### High priority (before open-sourcing)

- **Enforce IAM auth by default**

  - Remove permissive default: do not allow `ALLOWED_ROLE_ARNS="*"` fallback; require explicit allowlist and fail closed if unset.
  - Treat `SKIP_IAM_VALIDATION=true` as development-only; in release builds or non-dev envs refuse to start unless `ALLOW_INSECURE=true` is also set (and emit loud warnings).
  - Log effective auth settings at startup in structured form; never log secrets.

- **Gate WebSocket session on successful auth**

  - Require the first WS message to be an `IamAuth` handshake. Close connections that do not authenticate within 5–10s.
  - Maintain per-connection authenticated state and the validated `IamIdentity`; block all other message types until authenticated.
  - Enforce authorization for registration and routing based on the authenticated identity.

- **WebSocket origin and resource limits**

  - Check allowed `Origin`/`Host` during upgrade; reject cross-origin upgrades by default.
  - Use `WebSocketConfig` to set `max_message_size`, `max_frame_size`, and idle/read timeouts. Reasonable defaults: 1 MiB message, 256 KiB frame, 60s idle.
  - Implement rate limiting per connection (e.g., token bucket) and global connection throttles.

- **TLS and infra hygiene**

  - Parameterize ACM certs and TLS policies; avoid hardcoded ARNs. Make `TLS_CERT_ARN`/`TLS_POLICY` required CDK inputs.
  - Enforce TLS 1.2+ (prefer latest AWS ALB security policy) and enable HSTS for any HTTP endpoints sitting behind TLS terminators.
  - Restrict security groups: ALB ingress only from intended CIDRs; target group only from ALB SG.

- **Fail-safe boot configuration**
  - Refuse to start if critical security config is missing or insecure (empty role allowlist, skip flags enabled) unless explicitly overridden for development.

### Medium priority

- **Robust STS validation**

  - Keep using presigned `GetCallerIdentity` URLs with short TTL (≤60s); reject expired or malformed URLs.
  - Prefer robust XML parsing (avoid brittle substring search) or call the AWS SDK directly when credentials are present.
  - Log ARN/account/principal type only; never log the full presigned URL.

- **Connection lifecycle controls**

  - Enforce `max_connections` from config; bound concurrent WS sessions.
  - Close idle unauthenticated sessions quickly; require periodic heartbeats and enforce timeouts.

- **Proxy request validation**

  - Sanitize and bound paths/headers accepted for proxying. Apply allowlists for target hosts or paths where feasible.
  - Validate header size/count and reject oversized requests.

- **Observability and audit**

  - Structured logs (JSON option present) with connection IDs; audit auth attempts, registration, routing decisions, rate-limit events.
  - Export metrics (auth failures, rejected upgrades, connection churn) for dashboards/alerts.

- **Runtime hardening (containers)**

  - Run as non-root; add `USER` in Dockerfile. Use minimal/distroless base image.
  - Add container `HEALTHCHECK`. In ECS task defs: `readOnlyRootFilesystem`, `no-new-privileges`, drop Linux capabilities.

- **Supply chain and CI**

  - Add CI steps: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo audit`, and `cargo deny`.
  - Enable Dependabot (Rust + npm) and CodeQL. Sign releases and artifacts (SLSA/GitHub OIDC).

- **WAF and DDoS**

  - Attach AWS WAF to the ALB with managed rule sets and rate-based rules for port 8082.
  - Rely on AWS Shield Standard; tune thresholds.

- **Protocol polish**

  - Pin a `Sec-WebSocket-Protocol` (e.g., `mesh-v1`) and require it on both client and server.
  - Disable `permessage-deflate` unless required and assessed.

- **Internal Request Validation via Namespace Querying**
  - Detect internal requests by absence of ALB-specific headers (e.g., `x-amzn-trace-id`, `x-forwarded-for`).
  - For internal requests, query ECS (ListServices/DescribeServices) and Service Discovery (ListNamespaces/DiscoverInstances) to validate if the target host matches registered namespaces/services (e.g., `mesh-e2e.local` or subdomains).
  - Cache query results in-memory with a TTL (e.g., 60s) to reduce API overhead.
  - To mitigate potential spoofing of ALB headers (harder externally due to security groups, but possible internally), add format validation (e.g., check `x-amzn-trace-id` starts with "Root=1-").
  - Implementation: Add `aws-sdk-ecs` and `aws-sdk-servicediscovery` to Cargo.toml; grant IAM permissions to task role in infra/lib/stack.ts; update handle_alb_request in mesh/src/server/handlers/alb.rs with caching and queries.
  - Benefits: Prevents unauthorized internal access to mesh clients; complements external restrictions to registered public endpoints.
  - Risks: Adds latency (mitigated by caching); ensure queries are scoped to your cluster to avoid broad permissions.

### Low priority / documentation

- **Security program docs**
  - Add `SECURITY.md` (vulnerability reporting, supported versions), `THREAT_MODEL.md` (assumptions: ALB TLS termination, IAM-based auth), and `CONTRIBUTING.md` with “security in development” guidance.
  - Provide sample IAM least-privilege policies for ECS list/describe and STS operations.

### Quick wins to implement first

- Change auth defaults to fail-closed; remove `"*"` fallback for `ALLOWED_ROLE_ARNS` and require a non-empty allowlist.
- Require `IamAuth` as the first WS message; disconnect unauthenticated clients after 5–10s.
- Add Origin/Host checks and `WebSocketConfig` limits (size/time).
- Parameterize ACM cert and TLS policy in CDK; refuse deploy if unset.
- Run container as non-root and drop capabilities; add `HEALTHCHECK`.

### Proposed configuration flags and defaults

- **ALLOWED_ROLE_ARNS**: required (no default). Comma-separated allowlist of IAM ARNs/patterns.
- **SKIP_IAM_VALIDATION**: default `false`. Dev-only. If `true` in non-dev, require `ALLOW_INSECURE=true` to start.
- **ALLOW_INSECURE**: default `false`. Explicit guard to run with insecure settings (development only).
- **WEBSOCKET_ORIGIN_ALLOWLIST**: new. Comma-separated allowed origins/hosts for WS upgrade.
- **WEBSOCKET_MAX_MESSAGE_BYTES**: new. Default `1048576` (1 MiB).
- **WEBSOCKET_MAX_FRAME_BYTES**: new. Default `262144` (256 KiB).
- **WS_IDLE_TIMEOUT_SECONDS**: new. Default `60`.
- **MAX_CONNECTIONS**: already present; enforce at accept-time.
- **TLS_CERT_ARN / TLS_POLICY** (CDK): required inputs; policy recommended: `ELBSecurityPolicy-TLS13-1-2-2021-06` or latest.

### Tracking

Use this file to track progress. Consider adding checkboxes and linking PRs as items are implemented.
