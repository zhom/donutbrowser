#!/bin/bash
set -e

# Get the target triple from environment or use default
TARGET="${TARGET:-$(rustc -vV 2>/dev/null | sed -n 's|host: ||p' || echo "unknown")}"
MANIFEST_DIR="$(dirname "$0")"

# Determine binary name based on target
if [[ "$TARGET" == *"windows"* ]]; then
  BIN_NAME="donut-proxy.exe"
else
  BIN_NAME="donut-proxy"
fi

# Determine source path
HOST_TARGET=$(rustc -vV 2>/dev/null | sed -n 's|host: ||p' || echo "$TARGET")
if [[ "$TARGET" == "$HOST_TARGET" ]] || [[ "$TARGET" == "unknown" ]]; then
  # Native target - use debug or release based on profile
  if [[ "${PROFILE:-debug}" == "release" ]]; then
    SRC_DIR="$MANIFEST_DIR/target/release"
  else
    SRC_DIR="$MANIFEST_DIR/target/debug"
  fi
else
  # Cross-compilation target
  if [[ "${PROFILE:-debug}" == "release" ]]; then
    SRC_DIR="$MANIFEST_DIR/target/$TARGET/release"
  else
    SRC_DIR="$MANIFEST_DIR/target/$TARGET/debug"
  fi
fi

SOURCE="$SRC_DIR/$BIN_NAME"
DEST_DIR="$MANIFEST_DIR/binaries"
# Tauri expects the format: donut-proxy-{target} with hyphens
DEST_NAME="donut-proxy-$TARGET"
if [[ "$TARGET" == *"windows"* ]]; then
  DEST_NAME="$DEST_NAME.exe"
fi
DEST="$DEST_DIR/$DEST_NAME"

# Create binaries directory if it doesn't exist
mkdir -p "$DEST_DIR"

# Copy the binary if it exists
if [[ -f "$SOURCE" ]]; then
  cp "$SOURCE" "$DEST"
  echo "Copied $BIN_NAME to $DEST"
else
  echo "Warning: Binary not found at $SOURCE"
  echo "Building donut-proxy binary..."
  cd "$MANIFEST_DIR"
  BUILD_ARGS=("build" "--bin" "donut-proxy")
  if [[ -n "$PROFILE" ]] && [[ "$PROFILE" == "release" ]]; then
    BUILD_ARGS+=("--release")
  fi
  if [[ -n "$TARGET" ]] && [[ "$TARGET" != "unknown" ]] && [[ "$TARGET" != "$HOST_TARGET" ]]; then
    BUILD_ARGS+=("--target" "$TARGET")
  fi
  cargo "${BUILD_ARGS[@]}"
  if [[ -f "$SOURCE" ]]; then
    cp "$SOURCE" "$DEST"
    echo "Built and copied $BIN_NAME to $DEST"
  else
    echo "Error: Failed to build donut-proxy binary"
    exit 1
  fi
fi

