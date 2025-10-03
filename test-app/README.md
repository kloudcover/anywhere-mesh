# Rust Test App

A high-performance Rust web application built with Axum, designed for load testing and API testing scenarios.

## Features

- 🚀 **High Performance**: Built with Rust and Axum for maximum throughput
- 🔧 **Full API Coverage**: All endpoints from the original Python version
- 🐳 **Docker Ready**: Multi-stage build for optimal container size
- 🔄 **Load Testing Ready**: Stress test endpoints and performance monitoring
- 🛡️ **Type Safe**: Full Rust type safety and memory safety

## Quick Start

### Development

```bash
# Build and run locally
./build.sh
./run.sh

# Or use cargo directly
cargo build --release
./target/release/test-app
```

### Docker

```bash
# Build and run with Docker
docker build -t test-app-rust .
docker run -p 3000:3000 test-app-rust

# Or use docker-compose
docker-compose up --build
```

### Custom Port

```bash
# Environment variable
PORT=3001 ./run.sh

# Docker with custom port
docker run -p 3001:3000 -e PORT=3000 test-app-rust
```

## API Endpoints

| Endpoint              | Method | Description            |
| --------------------- | ------ | ---------------------- |
| `/`                   | GET    | HTML frontend          |
| `/health`             | GET    | Health check           |
| `/api/health`         | GET    | Health check (JSON)    |
| `/api/info`           | GET    | Service information    |
| `/api/echo`           | GET    | Echo request details   |
| `/api/time`           | GET    | Current timestamp      |
| `/api/headers`        | GET    | Request headers        |
| `/api/environment`    | GET    | Environment variables  |
| `/api/stress/<delay>` | GET    | Stress test with delay |

## Load Testing

This application is optimized for load testing scenarios:

```bash
# Example k6 test
k6 run -e TARGET_HOST=localhost -e BASE_URL=http://localhost:3000 load-test-script.js
```

## Build Structure

```
test-app/
├── src/main.rs          # Rust application source
├── Cargo.toml          # Rust dependencies
├── Cargo.lock          # Dependency lock file
├── Dockerfile          # Multi-stage Docker build
├── docker-compose.yml  # Docker Compose configuration
├── build.sh           # Build script
├── run.sh             # Run script with port management
├── .dockerignore      # Docker build exclusions
└── index.html         # Frontend HTML
```

## Performance Benefits

Compared to the original Python Flask version:

- **~10-50x faster** startup time
- **~5-15MB** smaller memory footprint
- **~10-100x higher** throughput
- **Zero** garbage collection pauses
- **Memory safe** by default

## Troubleshooting

### Port Already in Use

```bash
# Kill process on port 3000
lsof -ti :3000 | xargs kill -9

# Or use a different port
PORT=3001 ./run.sh
```

### Docker Build Issues

```bash
# Clean Docker cache
docker system prune -a

# Rebuild without cache
docker build --no-cache -t test-app-rust .
```

## Development

```bash
# Run in development mode
cargo run

# Run tests
cargo test

# Format code
cargo fmt

# Lint code
cargo clippy
```
