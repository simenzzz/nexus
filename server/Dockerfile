# syntax=docker/dockerfile:1.6
# Multi-stage build for the Nexus Rust backend. Optimized for layer caching:
# manifests + lockfile are copied first, then dependencies are built against a
# stub binary, then real sources land. Editing app code only invalidates the
# final `cargo build` layer.

# ─── Builder ───────────────────────────────────────────────────────────────
FROM rust:1.87-slim AS builder

WORKDIR /app

# Pkg-config and libssl-dev are required by some transitive crates (and
# kept here even though we don't link against system OpenSSL — the deps
# probe via pkg-config at build time).
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

# Cached dependency layer: fetch + compile deps against a stub binary that
# uses the real package name. Any change to Cargo.toml/Cargo.lock busts
# this layer; source-only edits don't.
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src \
 && printf 'fn main() {}\n' > src/main.rs \
 && cargo build --release --locked \
 && rm -rf src target/release/deps/nexus_server*

# Real sources, then build for real.
COPY src ./src
RUN cargo build --release --locked --bin nexus-server \
 && strip target/release/nexus-server

# ─── Runtime ───────────────────────────────────────────────────────────────
# Distroless cc-debian12: provides glibc + CA certs, no shell, no package
# manager, ~22 MB. The non-root variant runs as uid 65532 by default.
FROM gcr.io/distroless/cc-debian12:nonroot

WORKDIR /app

COPY --from=builder --chown=nonroot:nonroot /app/target/release/nexus-server /usr/local/bin/nexus-server

USER nonroot:nonroot

EXPOSE 3001

# No HEALTHCHECK at the image layer — distroless has no shell or `curl`.
# Compose-level healthchecks call /ready over HTTP from a sibling service.

ENTRYPOINT ["/usr/local/bin/nexus-server"]
