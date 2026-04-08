#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

GITHUB_REPO="zhom/donutbrowser"

# Load .env if running locally
if [[ -f "$REPO_ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$REPO_ROOT/.env"
  set +a
fi

# Validate required env vars
for var in R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_ENDPOINT_URL R2_BUCKET_NAME; do
  if [[ -z "${!var:-}" ]]; then
    echo "Error: $var is not set. Configure it in .env or export it."
    exit 1
  fi
done

# Export for AWS CLI
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION="auto"
# aws-cli v2.23+ sends integrity checksums by default; R2 rejects them
# with `Unauthorized` on ListObjectsV2. Disable.
export AWS_REQUEST_CHECKSUM_CALCULATION="WHEN_REQUIRED"
export AWS_RESPONSE_CHECKSUM_VALIDATION="WHEN_REQUIRED"

# Ensure endpoint URL has https:// prefix
R2_ENDPOINT="$R2_ENDPOINT_URL"
if [[ "$R2_ENDPOINT" != https://* ]]; then
  R2_ENDPOINT="https://$R2_ENDPOINT"
fi

# Determine version tag
if [[ $# -ge 1 ]]; then
  TAG="$1"
else
  echo "Fetching latest release tag..."
  TAG=$(gh release view --repo "$GITHUB_REPO" --json tagName -q .tagName)
  echo "Latest release: $TAG"
fi

VERSION="${TAG#v}"
echo "Publishing repositories for version $VERSION"

# Check required tools
for cmd in aws gh dpkg-scanpackages gzip createrepo_c; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "Error: $cmd is not installed."
    case "$cmd" in
      dpkg-scanpackages) echo "  Install with: sudo apt-get install dpkg-dev" ;;
      createrepo_c) echo "  Install with: sudo apt-get install createrepo-c" ;;
      aws) echo "  Install with: pip install awscli" ;;
      gh) echo "  Install with: https://cli.github.com/" ;;
    esac
    exit 1
  fi
done

PACKAGES_DIR="$WORK_DIR/packages"
REPO_DIR="$WORK_DIR/repo"
mkdir -p "$PACKAGES_DIR" "$REPO_DIR"

# ---------------------------------------------------------------------------
# Download .deb and .rpm from GitHub release
# ---------------------------------------------------------------------------
echo ""
echo "==> Downloading packages from GitHub release $TAG..."
gh release download "$TAG" \
  --repo "$GITHUB_REPO" \
  --pattern "*.deb" \
  --dir "$PACKAGES_DIR"
gh release download "$TAG" \
  --repo "$GITHUB_REPO" \
  --pattern "*.rpm" \
  --dir "$PACKAGES_DIR"

echo "Downloaded:"
ls -lh "$PACKAGES_DIR/"

# ---------------------------------------------------------------------------
# DEB repository
# ---------------------------------------------------------------------------
echo ""
echo "==> Building DEB repository..."

DEB_DIR="$REPO_DIR/deb"
mkdir -p "$DEB_DIR/pool/main"
mkdir -p "$DEB_DIR/dists/stable/main/binary-amd64"
mkdir -p "$DEB_DIR/dists/stable/main/binary-arm64"

# Pull existing pool from R2 (incremental)
echo "  Syncing existing DEB pool from R2..."
aws s3 sync "s3://${R2_BUCKET_NAME}/deb/pool" "$DEB_DIR/pool" \
  --endpoint-url "$R2_ENDPOINT" 2>/dev/null || true

# Copy new .deb files into pool
for deb in "$PACKAGES_DIR"/*.deb; do
  [[ -f "$deb" ]] || continue
  cp "$deb" "$DEB_DIR/pool/main/"
done

# Generate Packages and Packages.gz for each arch
for arch in amd64 arm64; do
  echo "  Generating Packages for $arch..."
  BINARY_DIR="$DEB_DIR/dists/stable/main/binary-${arch}"

  # dpkg-scanpackages needs to run from the repo root
  # and needs paths relative to that root
  (cd "$DEB_DIR" && dpkg-scanpackages --arch "$arch" pool/main) \
    > "$BINARY_DIR/Packages"

  gzip -9c "$BINARY_DIR/Packages" > "$BINARY_DIR/Packages.gz"

  echo "    $(grep -c '^Package:' "$BINARY_DIR/Packages" 2>/dev/null || echo 0) package(s)"
done

# Generate Release file
echo "  Generating Release file..."
{
  echo "Origin: Donut Browser"
  echo "Label: Donut Browser"
  echo "Suite: stable"
  echo "Codename: stable"
  echo "Architectures: amd64 arm64"
  echo "Components: main"
  echo "Date: $(date -u '+%a, %d %b %Y %H:%M:%S UTC')"
  echo "MD5Sum:"
  for arch in amd64 arm64; do
    for file in "main/binary-${arch}/Packages" "main/binary-${arch}/Packages.gz"; do
      filepath="$DEB_DIR/dists/stable/$file"
      if [[ -f "$filepath" ]]; then
        size=$(wc -c < "$filepath")
        md5=$(md5sum "$filepath" | awk '{print $1}')
        printf " %s %8d %s\n" "$md5" "$size" "$file"
      fi
    done
  done
  echo "SHA256:"
  for arch in amd64 arm64; do
    for file in "main/binary-${arch}/Packages" "main/binary-${arch}/Packages.gz"; do
      filepath="$DEB_DIR/dists/stable/$file"
      if [[ -f "$filepath" ]]; then
        size=$(wc -c < "$filepath")
        sha256=$(sha256sum "$filepath" | awk '{print $1}')
        printf " %s %8d %s\n" "$sha256" "$size" "$file"
      fi
    done
  done
} > "$DEB_DIR/dists/stable/Release"

echo "  DEB Release file created."

# ---------------------------------------------------------------------------
# RPM repository
# ---------------------------------------------------------------------------
echo ""
echo "==> Building RPM repository..."

RPM_DIR="$REPO_DIR/rpm"
mkdir -p "$RPM_DIR/x86_64"
mkdir -p "$RPM_DIR/aarch64"

# Pull existing RPMs from R2 (incremental)
echo "  Syncing existing RPM packages from R2..."
aws s3 sync "s3://${R2_BUCKET_NAME}/rpm/x86_64" "$RPM_DIR/x86_64" \
  --endpoint-url "$R2_ENDPOINT" --exclude "repodata/*" 2>/dev/null || true
aws s3 sync "s3://${R2_BUCKET_NAME}/rpm/aarch64" "$RPM_DIR/aarch64" \
  --endpoint-url "$R2_ENDPOINT" --exclude "repodata/*" 2>/dev/null || true

# Copy new .rpm files into arch directories
for rpm in "$PACKAGES_DIR"/*.rpm; do
  [[ -f "$rpm" ]] || continue
  filename=$(basename "$rpm")
  if [[ "$filename" == *x86_64* ]]; then
    cp "$rpm" "$RPM_DIR/x86_64/"
  elif [[ "$filename" == *aarch64* ]]; then
    cp "$rpm" "$RPM_DIR/aarch64/"
  fi
done

# Generate repodata using createrepo_c
# We point createrepo_c at the top-level rpm dir so it indexes all subdirs
echo "  Generating RPM repodata..."
createrepo_c --update "$RPM_DIR"

echo "  RPM repodata created."

# ---------------------------------------------------------------------------
# Upload to R2
# ---------------------------------------------------------------------------
echo ""
echo "==> Uploading DEB repository to R2..."
aws s3 sync "$DEB_DIR/dists" "s3://${R2_BUCKET_NAME}/deb/dists" \
  --endpoint-url "$R2_ENDPOINT" --delete
aws s3 sync "$DEB_DIR/pool" "s3://${R2_BUCKET_NAME}/deb/pool" \
  --endpoint-url "$R2_ENDPOINT"

echo "==> Uploading RPM repository to R2..."
aws s3 sync "$RPM_DIR" "s3://${R2_BUCKET_NAME}/rpm" \
  --endpoint-url "$R2_ENDPOINT"

# ---------------------------------------------------------------------------
# Verify
# ---------------------------------------------------------------------------
echo ""
echo "==> Verifying upload..."
echo "DEB dists/stable/:"
aws s3 ls "s3://${R2_BUCKET_NAME}/deb/dists/stable/" \
  --endpoint-url "$R2_ENDPOINT" 2>/dev/null || echo "  (empty or not accessible)"
echo "DEB pool/main/:"
aws s3 ls "s3://${R2_BUCKET_NAME}/deb/pool/main/" \
  --endpoint-url "$R2_ENDPOINT" 2>/dev/null || echo "  (empty or not accessible)"
echo "RPM repodata/:"
aws s3 ls "s3://${R2_BUCKET_NAME}/rpm/repodata/" \
  --endpoint-url "$R2_ENDPOINT" 2>/dev/null || echo "  (empty or not accessible)"

echo ""
echo "Done! Repository published for $TAG"
echo ""
echo "Users can add the DEB repo with:"
echo "  echo 'deb [trusted=yes] https://repo.donutbrowser.com/deb stable main' | sudo tee /etc/apt/sources.list.d/donutbrowser.list"
echo "  sudo apt update && sudo apt install donut"
echo ""
echo "Users can add the RPM repo with:"
echo "  sudo tee /etc/yum.repos.d/donutbrowser.repo << 'EOF'"
echo "  [donutbrowser]"
echo "  name=Donut Browser"
echo "  baseurl=https://repo.donutbrowser.com/rpm"
echo "  enabled=1"
echo "  gpgcheck=0"
echo "  EOF"
echo "  sudo dnf install Donut"
