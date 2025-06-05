#!/bin/bash

# Determine file extension based on platform
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" || "$OSTYPE" == "cygwin" ]]; then
    EXT=".exe"
else
    EXT=""
fi

# Get Rust target triple
RUST_INFO=$(rustc -vV)
TARGET_TRIPLE=$(echo "$RUST_INFO" | grep -o 'host: [^ ]*' | cut -d' ' -f2)

# Check if target triple was found
if [ -z "$TARGET_TRIPLE" ]; then
    echo "Failed to determine platform target triple" >&2
    exit 1
fi

# Rename the file
mv "nodecar/dist/nodecar${EXT}" "src-tauri/binaries/nodecar-${TARGET_TRIPLE}${EXT}"