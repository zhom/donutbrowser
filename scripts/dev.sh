#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Get the root directory of the project
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
SYNC_DIR="$ROOT_DIR/donut-sync"

# Track PIDs for cleanup
SYNC_PID=""
TAURI_PID=""
SHUTTING_DOWN=false

cleanup() {
  if [ "$SHUTTING_DOWN" = true ]; then
    return
  fi
  SHUTTING_DOWN=true

  echo -e "\n${YELLOW}Shutting down services...${NC}"

  # Kill Tauri if running
  if [ -n "$TAURI_PID" ] && kill -0 "$TAURI_PID" 2>/dev/null; then
    echo -e "${BLUE}Stopping Tauri...${NC}"
    kill "$TAURI_PID" 2>/dev/null || true
  fi

  # Kill sync backend if running
  if [ -n "$SYNC_PID" ] && kill -0 "$SYNC_PID" 2>/dev/null; then
    echo -e "${BLUE}Stopping sync backend...${NC}"
    kill "$SYNC_PID" 2>/dev/null || true
  fi

  # Stop MinIO container
  echo -e "${BLUE}Stopping MinIO container...${NC}"
  cd "$SYNC_DIR" && docker compose down 2>/dev/null || true

  # Wait for processes to finish
  wait 2>/dev/null || true

  echo -e "${GREEN}Cleanup complete.${NC}"
}

trap cleanup EXIT INT TERM

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Donut Browser Development Environment${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Check prerequisites
echo -e "${YELLOW}Checking prerequisites...${NC}"

if ! command -v docker &> /dev/null; then
  echo -e "${RED}Error: docker is not installed${NC}"
  exit 1
fi

if ! command -v pnpm &> /dev/null; then
  echo -e "${RED}Error: pnpm is not installed${NC}"
  exit 1
fi

echo -e "${GREEN}Prerequisites OK${NC}"
echo ""

# Start MinIO container
echo -e "${YELLOW}Starting MinIO (S3) container...${NC}"
cd "$SYNC_DIR"
docker compose up -d

# Wait for MinIO to be healthy
echo -e "${YELLOW}Waiting for MinIO to be healthy...${NC}"
MAX_RETRIES=30
RETRY_COUNT=0
while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
  if curl -sf http://localhost:8987/minio/health/live > /dev/null 2>&1; then
    echo -e "${GREEN}MinIO is ready!${NC}"
    break
  fi
  RETRY_COUNT=$((RETRY_COUNT + 1))
  if [ $RETRY_COUNT -eq $MAX_RETRIES ]; then
    echo -e "${RED}MinIO failed to start within timeout${NC}"
    exit 1
  fi
  sleep 1
done
echo ""

# Install sync backend dependencies if needed
if [ ! -d "$SYNC_DIR/node_modules" ]; then
  echo -e "${YELLOW}Installing sync backend dependencies...${NC}"
  cd "$SYNC_DIR" && pnpm install
fi

# Start sync backend in background
echo -e "${YELLOW}Starting sync backend...${NC}"
cd "$SYNC_DIR"
pnpm start:dev &
SYNC_PID=$!

# Wait for sync backend to be ready
echo -e "${YELLOW}Waiting for sync backend to be ready...${NC}"
MAX_RETRIES=60
RETRY_COUNT=0
while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
  if curl -sf http://localhost:12342/health > /dev/null 2>&1; then
    echo -e "${GREEN}Sync backend is ready!${NC}"
    break
  fi
  # Check if process is still running
  if ! kill -0 "$SYNC_PID" 2>/dev/null; then
    echo -e "${RED}Sync backend process died${NC}"
    exit 1
  fi
  RETRY_COUNT=$((RETRY_COUNT + 1))
  if [ $RETRY_COUNT -eq $MAX_RETRIES ]; then
    echo -e "${RED}Sync backend failed to start within timeout${NC}"
    exit 1
  fi
  sleep 1
done
echo ""

# Start Tauri app in background
echo -e "${YELLOW}Starting Tauri development server...${NC}"
echo -e "${BLUE}Frontend: http://localhost:12341${NC}"
echo -e "${BLUE}Sync Backend: http://localhost:12342${NC}"
echo -e "${BLUE}MinIO Console: http://localhost:8988${NC}"
echo ""
cd "$ROOT_DIR"
pnpm tauri dev &
TAURI_PID=$!

# Monitor all processes - exit if any dies
echo -e "${YELLOW}Monitoring processes (Ctrl+C to stop all)...${NC}"
while true; do
  # Check if sync backend died
  if ! kill -0 "$SYNC_PID" 2>/dev/null; then
    echo -e "${RED}Sync backend crashed!${NC}"
    exit 1
  fi

  # Check if Tauri died
  if ! kill -0 "$TAURI_PID" 2>/dev/null; then
    echo -e "${RED}Tauri exited!${NC}"
    exit 1
  fi

  sleep 2
done
