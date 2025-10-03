# syntax=docker/dockerfile:1.5
FROM rust:1.86 as builder

WORKDIR /usr/src/app

# Ensure Cargo home is consistent for caching
ENV CARGO_HOME=/usr/local/cargo

# Copy only Cargo files first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY mesh/ ./mesh/


# Build the actual application and persist binary outside cache-mounted target
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/usr/src/app/target \
    cargo build --release --bin mesh && \
    mkdir -p /out && \
    cp /usr/src/app/target/release/mesh /out/mesh

# Runtime image - using debian slim for better compatibility
FROM debian:12-slim

# Install minimal required packages
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary persisted from the builder stage
COPY --from=builder /out/mesh /usr/local/bin/mesh

# Ensure binary is executable
RUN chmod +x /usr/local/bin/mesh

# Set the startup command
ENTRYPOINT ["/usr/local/bin/mesh"]
