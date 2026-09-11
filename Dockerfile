FROM node:24-bookworm-slim AS frontend-builder
WORKDIR /build/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:1.96-bookworm AS rust-builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml backend/Cargo.toml
COPY backend/src backend/src
RUN cargo build --release --locked --package canvas-pocket-server

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 canvas
WORKDIR /app
COPY --from=rust-builder /build/target/release/canvas-pocket-server /usr/local/bin/canvas-pocket-server
COPY --from=frontend-builder /build/frontend/dist /app/web
RUN mkdir -p /data && chown -R canvas:canvas /data /app
USER canvas
ENV BIND=0.0.0.0:8080 \
    DATA_DIR=/data \
    WEB_DIST=/app/web \
    COOKIE_SECURE=true
EXPOSE 8080
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:8080/api/health >/dev/null || exit 1
ENTRYPOINT ["canvas-pocket-server"]

