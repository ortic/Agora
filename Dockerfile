# syntax=docker/dockerfile:1.7
# ---- frontend ----
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# ---- backend ----
FROM rust:1.96-bookworm AS server
# SAML 2.0 (`--build-arg FEATURES=saml`) links libxmlsec1 for XML signatures
ARG FEATURES=""
RUN if [ -n "$FEATURES" ]; then apt-get update && apt-get install -y --no-install-recommends libxml2-dev libxmlsec1-dev libclang-dev clang pkg-config && rm -rf /var/lib/apt/lists/*; fi
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# the web app is embedded into the binary
COPY --from=web /web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p agora --features "$FEATURES" && cp target/release/agora /agora

# ---- runtime ----
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libxml2 libxmlsec1 libxmlsec1-openssl && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 agora
COPY --from=server /agora /usr/local/bin/agora
USER agora
ENV AGORA_BIND=0.0.0.0:8790 \
    AGORA_STATIC_DIR=/nonexistent
EXPOSE 8790
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD ["agora", "healthcheck"]
ENTRYPOINT ["agora"]
CMD ["serve"]
