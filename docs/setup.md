# Nexus — Setup Guide

## Option A: Docker (Recommended)

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

## Option B: Local Development

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
# 2. Start SurrealDB only
docker compose up surrealdb -d
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
- **Docker build fails on Rust**: The Rust build needs ~2GB RAM. Ensure Docker has enough memory allocated.
- **Cargo build fails locally**: Ensure latest stable Rust: `rustup update stable`.
- **npm install fails**: Ensure Node >= 20: `node --version`.
