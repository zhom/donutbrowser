# Self-Hosting Donut Sync

Donut Sync is the synchronization server for Donut Browser. It allows you to sync your profiles, proxies, and groups across multiple devices. This guide covers how to self-host it using Docker.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/)
- An S3-compatible object storage (MinIO included by default, or use AWS S3, Cloudflare R2, etc.)

## Quick Start

### 1. Create a `docker-compose.yml`

```yaml
services:
  donut-sync:
    image: donutbrowser/donut-sync:latest
    ports:
      - "3929:3929"
    environment:
      - SYNC_TOKEN=your-secret-token-here
      - PORT=3929
      - S3_ENDPOINT=http://minio:9000
      - S3_REGION=us-east-1
      - S3_ACCESS_KEY_ID=minioadmin
      - S3_SECRET_ACCESS_KEY=minioadmin
      - S3_BUCKET=donut-sync
      - S3_FORCE_PATH_STYLE=true
    depends_on:
      minio:
        condition: service_healthy

  minio:
    image: minio/minio:latest
    ports:
      - "9000:9000"
      - "9001:9001"
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    command: server /data --console-address ":9001"
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:9000/minio/health/live"]
      interval: 5s
      timeout: 5s
      retries: 5
    volumes:
      - minio_data:/data

volumes:
  minio_data:
```

### 2. Start the services

```bash
docker compose up -d
```

### 3. Verify the server is running

```bash
# Health check
curl http://localhost:3929/health
# Expected: {"status":"ok"}

# Readiness check (verifies S3 connectivity)
curl http://localhost:3929/readyz
# Expected: {"status":"ready","s3":true}
```

## Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `SYNC_TOKEN` | Yes | - | Bearer token used to authenticate requests from Donut Browser clients |
| `PORT` | No | `3929` | Port the sync server listens on |
| `S3_ENDPOINT` | No | - | S3-compatible endpoint URL (e.g., `http://minio:9000` or `https://s3.amazonaws.com`) |
| `S3_REGION` | No | `us-east-1` | S3 region |
| `S3_ACCESS_KEY_ID` | Yes | - | S3 access key |
| `S3_SECRET_ACCESS_KEY` | Yes | - | S3 secret key |
| `S3_BUCKET` | No | `donut-sync` | S3 bucket name for storing sync data |
| `S3_FORCE_PATH_STYLE` | No | `false` | Set to `true` for MinIO and other S3-compatible services that use path-style URLs |

## Using External S3 Storage

Instead of running MinIO, you can use any S3-compatible storage service. Remove the `minio` service from `docker-compose.yml` and update the environment variables:

### AWS S3

```yaml
services:
  donut-sync:
    image: donutbrowser/donut-sync:latest
    ports:
      - "3929:3929"
    environment:
      - SYNC_TOKEN=your-secret-token-here
      - S3_REGION=us-east-1
      - S3_ACCESS_KEY_ID=your-aws-access-key
      - S3_SECRET_ACCESS_KEY=your-aws-secret-key
      - S3_BUCKET=your-bucket-name
```

### Cloudflare R2

```yaml
services:
  donut-sync:
    image: donutbrowser/donut-sync:latest
    ports:
      - "3929:3929"
    environment:
      - SYNC_TOKEN=your-secret-token-here
      - S3_ENDPOINT=https://<account-id>.r2.cloudflarestorage.com
      - S3_REGION=auto
      - S3_ACCESS_KEY_ID=your-r2-access-key
      - S3_SECRET_ACCESS_KEY=your-r2-secret-key
      - S3_BUCKET=your-bucket-name
      - S3_FORCE_PATH_STYLE=true
```

### Other S3-Compatible Services

Any service that implements the S3 API (e.g., Backblaze B2, DigitalOcean Spaces, Wasabi) can be used. Set `S3_ENDPOINT` to the service's endpoint URL and `S3_FORCE_PATH_STYLE=true` if required by the provider.

## Configuring the Donut Browser Client

1. Open Donut Browser
2. Click the sync icon in the header to open the Sync Configuration dialog
3. Enter the **Server URL** (e.g., `http://your-server:3929`)
4. Enter the **Sync Token** (the value you set for `SYNC_TOKEN`)
5. Click **Save**

Once configured, you can enable sync on individual profiles, proxies, and groups.

## Health Check Endpoints

| Endpoint | Description |
|---|---|
| `GET /health` | Basic health check. Returns `{"status":"ok"}` if the server is running. |
| `GET /readyz` | Readiness check. Verifies S3 connectivity. Returns `{"status":"ready","s3":true}` or HTTP 503 if S3 is unreachable. |

## Security Considerations

- **Use a strong `SYNC_TOKEN`**: Generate a random token (e.g., `openssl rand -hex 32`) and keep it secret.
- **HTTPS**: In production, place a reverse proxy (e.g., Nginx, Caddy, Traefik) in front of Donut Sync to terminate TLS. The sync token is sent as a Bearer token in the `Authorization` header and should not be transmitted over plain HTTP.
- **Network isolation**: If running on a VPS, consider restricting access to the sync port using firewall rules or binding only to localhost behind a reverse proxy.
- **S3 credentials**: Use dedicated IAM credentials with minimal permissions (read/write to the sync bucket only).

### Example: Caddy Reverse Proxy

```
sync.yourdomain.com {
    reverse_proxy localhost:3929
}
```

### Example: Nginx Reverse Proxy

```nginx
server {
    listen 443 ssl;
    server_name sync.yourdomain.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    location / {
        proxy_pass http://localhost:3929;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```
