# ==============================================================================
# IRCord - Hybride IRC-Discord AI Bot Daemon
# Multi-stage Dockerfile voor ultralichte Alpine footprint (< 25 MB)
# ==============================================================================

# STAGE 1: Bouwomgeving
FROM rust:alpine AS builder

RUN apk add --no-cache \
    musl-dev \
    sqlite-dev \
    openssl-dev \
    openssl-libs-static \
    pkgconfig \
    make \
    git

WORKDIR /usr/src/ircord

# Kopieer dependency manifests om layers efficiënt te cachen
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src migrations && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release || true && \
    rm -rf src

# Kopieer de daadwerkelijke broncode en migraties
COPY . .

# Compileer geoptimaliseerde release binary
ENV SQLX_OFFLINE=true
RUN cargo build --release --locked || cargo build --release

# STAGE 2: Minimale Productie Runtime (< 25 MB)
FROM alpine:latest AS runtime

RUN apk add --no-cache \
    ca-certificates \
    sqlite-libs \
    tzdata \
    curl

WORKDIR /app

# Kopieer gecompileerde binary en benodigde assets
COPY --from=builder /usr/src/ircord/target/release/ircord /app/ircord
COPY --from=builder /usr/src/ircord/migrations /app/migrations
COPY config.toml /app/config.toml
COPY .env.example /app/.env.example

# Creëer data- en scripts directory
RUN mkdir -p /app/data /app/scripts

# Stel omgevingsvariabelen in
ENV RUST_LOG=info,ircord=debug
ENV DATABASE_URL="sqlite:///app/data/ircord.db"

# Exposeer metrics & webhook poort
EXPOSE 9090

# Healthcheck via Axum /health endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:9090/health || exit 1

# Start daemon
ENTRYPOINT ["/app/ircord"]
