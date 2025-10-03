## k6 Load Testing for Anywhere Mesh

This suite targets the ALB-exposed ingress at your domain. Example domain from infra params: `e2e.clustermaestro.com`.

### Install k6

- Homebrew: `brew install k6`
- Docker: `docker run --rm -i grafana/k6 run - < load-test/http-smoke.js`

### Environment

Create a `.env` or export variables before running (see `env.example`):

```
export TARGET_HOST="your.domain"
export BASE_URL="https://${TARGET_HOST}"
export WS_URL="wss://${TARGET_HOST}:8082"
export TEST_TAGS="env:dev,service:mesh"
```

### Scenarios

- http-smoke.js: very light check of health and info endpoints
- http-baseline.js: steady RPS against `/api/health`, `/api/info`, `/api/echo`, `/api/time`
- http-spike.js: rapid ramp-up spike to shake routing and backpressure
- http-soak.js: sustained moderate load to observe stability and memory growth
- ws-connect.js (optional): opens and maintains WebSocket connections to validate tunnel capacity

### Run

Examples (local k6):

```
cd load-test
k6 run http-smoke.js
k6 run http-baseline.js
k6 run http-spike.js
k6 run http-soak.js
k6 run ws-connect.js
```

With Docker (no k6 install needed):

```
docker run --rm -i \
  -e BASE_URL -e WS_URL -e TARGET_HOST -e TEST_TAGS \
  -v "$(pwd)":/scripts \
  grafana/k6 run /scripts/load-test/http-baseline.js
```

### Notes

- HTTP requests include `Host: ${TARGET_HOST}` to exercise mesh routing.
- Adjust thresholds and stages to match your SLOs.
