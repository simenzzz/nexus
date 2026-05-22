# syntax=docker/dockerfile:1.6
# Multi-stage SvelteKit (adapter-node) build.

# ─── Builder ───────────────────────────────────────────────────────────────
FROM node:24-alpine AS builder

WORKDIR /app

# Install deps with a deterministic, full install (lockfile required).
COPY package.json package-lock.json* ./
RUN npm ci

# Source + build. SvelteKit emits `build/` per the adapter-node config.
COPY . .
RUN npm run build

# Prune to production deps only — keeps the runtime image lean.
RUN npm prune --omit=dev

# ─── Runtime ───────────────────────────────────────────────────────────────
FROM node:24-alpine AS runtime

WORKDIR /app

ENV NODE_ENV=production \
    PORT=3000 \
    HOST=0.0.0.0

# node:24-alpine ships an unprivileged `node` user (uid 1000).
COPY --from=builder --chown=node:node /app/build ./build
COPY --from=builder --chown=node:node /app/package.json ./
COPY --from=builder --chown=node:node /app/node_modules ./node_modules

USER node

EXPOSE 3000

# In-process healthcheck — avoids pulling curl. Hits the root path; treats
# any 2xx/3xx as healthy.
HEALTHCHECK --interval=15s --timeout=5s --retries=5 \
    CMD node -e "fetch('http://127.0.0.1:3000/').then(r=>process.exit(r.ok||r.status<400?0:1)).catch(()=>process.exit(1))"

CMD ["node", "build"]
