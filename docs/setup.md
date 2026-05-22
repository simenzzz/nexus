# Nexus — Setup Guide

## Option A: Local Docker

The entire stack runs from a single command. No Rust, Node, or SurrealDB installation needed.

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and Docker Compose
- [Git](https://git-scm.com/)

### Steps

```bash
# 1. Clone and configure
git clone <repo-url> && cd nexus
cp .env.example .env
# Edit .env to set a real JWT_SECRET

# 2. Start everything
docker compose up --build

# First build takes a few minutes (compiling Rust, installing npm deps).
# Subsequent starts use cached layers and are fast.
```

Once running:
- **Frontend**: http://localhost:3000
- **Backend API**: http://localhost:3001
- **SurrealDB**: ws://localhost:8000

```bash
# Stop all services
docker compose down

# Stop and remove persisted data
docker compose down -v
```

## Option B: Production Docker on a VPS

Use this path for a public deployment behind a real domain. Caddy is the only
public ingress and provisions HTTPS automatically; the client, API, Redis, and
SurrealDB stay on the private Compose network.

### Prerequisites

- Linux VPS with Docker and Docker Compose
- DNS `A`/`AAAA` record for your domain pointing at the VPS
- Host firewall allowing only SSH, HTTP 80, and HTTPS 443

### Steps

```bash
# 1. Configure production secrets
cp .env.example .env
```

Set these values in `.env`:

```bash
NEXUS_ENV=production
DOMAIN=app.example.com
ACME_EMAIL=admin@example.com
JWT_SECRET=<openssl rand -hex 32>
SURREAL_USER=nexus-admin
SURREAL_PASS=<openssl rand -hex 24>
REDIS_PASSWORD=<openssl rand -hex 24>
SECURE_COOKIES=true
CORS_ORIGIN=https://app.example.com
```

Leave `PUBLIC_API_URL` and `PUBLIC_WS_URL` at their example values or blank;
the production override clears them so the browser uses same-origin `/api`
and `/ws` through Caddy.

```bash
# 2. Confirm the merged production config exposes only Caddy ports
docker compose -f docker-compose.yml -f docker-compose.prod.yml config

# 3. Build and start the stack
docker compose -f docker-compose.yml -f docker-compose.prod.yml up -d --build

# 4. Check service readiness
curl -fsS https://app.example.com/health
curl -fsS https://app.example.com/ready
```

Expected public ports in the merged config are only `80:80`, `443:443`, and
`443:443/udp` on the `caddy` service.

## Option C: Local Development

For faster iteration with hot-reload on both frontend and backend.

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) >= 20 and npm >= 10
- [Docker](https://docs.docker.com/get-docker/) (for SurrealDB only)

### Steps

```bash
# 1. Configure environment
cp .env.example .env
# Change SURREAL_URL to ws://localhost:8000 for local dev
```

```bash
# 2. Start SurrealDB and run the schema bootstrap
docker compose up surrealdb surreal-init
```

```bash
# 3. Backend (in one terminal)
cd server
cargo run
# Server starts on http://localhost:3001
```

```bash
# 4. Frontend (in another terminal)
cd client
npm install
npm run dev
# App available at http://localhost:5173 (dev mode with hot-reload)
```

## Dependency Management

- **Rust dependencies**: Defined in `server/Cargo.toml`, locked in `server/Cargo.lock`
- **Node dependencies**: Defined in `client/package.json`, locked in `client/package-lock.json`
- **System services**: Defined in `docker-compose.yml`

## Troubleshooting

- **Port 8000 in use**: Another service is using the SurrealDB port. Stop it or change the port in `docker-compose.yml` and `.env`.
- **Port 3001 in use**: Change `SERVER_PORT` in `.env`.
- **Production config exposes Redis/DB/API ports**: Ensure the prod override is included: `docker compose -f docker-compose.yml -f docker-compose.prod.yml config`.
- **Production server exits immediately**: Check `.env` for placeholder values. Production mode rejects `root` Surreal credentials, short secrets, insecure cookies, and missing CORS origin.
- **Login works but refresh/logout fails**: Confirm the site is loaded over `https://DOMAIN` and browser cookies include `__Host-refresh_token` and `__Host-csrf_token`.
- **Docker build fails on Rust**: The Rust build needs ~2GB RAM. Ensure Docker has enough memory allocated.
- **Cargo build fails locally**: Ensure latest stable Rust: `rustup update stable`.
- **npm install fails**: Ensure Node >= 20: `node --version`.
