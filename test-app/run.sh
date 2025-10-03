#!/bin/bash

set -e

echo "🚀 Starting Rust test-app..."

# Check if port is provided
PORT="${PORT:-3000}"

# Kill any existing process on the port
echo "🔍 Checking if port $PORT is in use..."
if lsof -i :$PORT > /dev/null 2>&1; then
    echo "⚠️  Port $PORT is in use. Attempting to free it..."
    lsof -ti :$PORT | xargs kill -9 2>/dev/null || true
    sleep 2
fi

# Set environment variable
export PORT=$PORT

# Run the application
echo "🌐 Starting server on port $PORT..."
echo "📊 Health check: http://localhost:$PORT/api/health"
echo "🛑 Press Ctrl+C to stop"
echo ""

./target/release/test-app
