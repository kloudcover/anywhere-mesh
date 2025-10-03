# Repository Guidelines

## Project Structure & Module Organization

- `mesh/`: Rust CLI `ecs`; `src/server`, `src/client`, and `src/common` hold ingress, agent, and shared logic.
- `test-app/`: Axum sample for smoke tests and docker-compose demos.
- `infra/`: AWS CDK TypeScript stack (`lib/infra-stack.ts`, `bin/infra.ts`) for ALB, ECS, and networking.
- `load-test/`: k6 scenarios (`http-*.js`, `ws-connect.js`) plus `env.example` for baseline variables.
- `config/`: Client YAML templates; copy before editing to avoid committing secrets.

## Build, Test & Development Commands

- `cargo build --release --bin mesh`: compile the mesh CLI.
- `cargo run --bin mesh -- server|client`: start ingress or client locally.
- `cargo test`: run Rust unit tests.
- `cargo fmt -- --check` & `cargo clippy --all-targets -- -D warnings`: enforce formatting and lint gates.
- `task build`, `task e2e`, `task load-test`: docker build, CDK deploy, and combined load tests.
- `npm install && npm run build` in `infra/`: compile CDK stacks; `npm test` runs Jest specs.
- `k6 run load-test/http-smoke.js`: quick regression; define `TARGET_HOST` or `BASE_URL` for remote targets.

## Coding Style & Naming Conventions

- Rust edition 2021 with default `rustfmt`; 4-space indents, snake_case modules, CamelCase types.
- Prefer `anyhow::Context` on errors and reuse existing `tracing` spans for observability.
- TypeScript constructs remain PascalCase with named exports; keep CDK prop objects immutable.
- k6 scripts stay lowercase-hyphenated; environment variables uppercase (e.g., `MESH_HOST`).

## Testing Guidelines

- Add Rust tests next to source modules; helper functions ending `_tests` keep intent clear.
- Run `cargo test`, `cargo fmt`, and `cargo clippy` before pushing; ensure server/client coverage when networking changes.
- Validate infra with `npm test` and `cdk synth`; capture notable diff output in PR notes.
- Exercise load paths with `TARGET_HOST=... k6 run load-test/http-baseline.js`; stop docker-compose demos once complete.

## Commit & Pull Request Guidelines

- Use short, imperative commit subjects (e.g., `Add WS reconnect guard`) with optional body for rationale and validation steps.
- Keep commits focused; exclude generated artifacts like `target/` or build caches.
- PRs should summarize scope, reference issues or Taskfile targets, and list validation commands.
- Attach logs, screenshots, or k6 metrics when behavior changes; request infra reviewers when CDK stacks change.

## Security & Configuration Tips

- Do not commit credentials favor `aws-vault` or AWS SSO sessions.
- Rotate sample tokens regularly and note changes in PR descriptions.
- Shut down docker-compose environments with `docker compose down` to clear ephemeral credentials.
