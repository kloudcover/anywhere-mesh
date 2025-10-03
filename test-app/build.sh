#!/bin/bash

set -e

echo "🏗️  Building Rust test-app..."

# Clean previous build
echo "🧹 Cleaning previous build..."
cargo clean

# Build the application
echo "🔨 Building release version..."
cargo build --release

echo "✅ Build completed successfully!"
echo "🚀 Run with: ./target/release/test-app"
echo "🐳 Or build Docker: docker build -t test-app-rust ."
