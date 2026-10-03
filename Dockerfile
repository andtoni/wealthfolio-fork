# Global build args
ARG RUST_IMAGE=rust:1.91-alpine@sha256:45c1c35cd364b8055e9e86f8ecd3e8c874b2dcb658d8a4f94b5d111aa0d651a2
ARG SOURCE_REVISION=unknown
ARG CACHE_NAMESPACE=wealthfolio-v1

# Stage 1: build frontend
# Use --platform=$BUILDPLATFORM to run on the native runner (fast)
FROM --platform=$BUILDPLATFORM node:24.18.1-alpine@sha256:f70403e87646dc51b45295f4b8b70cdad0b63d2297c4c9899119b03f7af7a6b3 AS frontend
ARG CACHE_NAMESPACE

# Wealthfolio Connect configuration (baked into JS bundle at build time)
# Pass via --build-arg to enable; omit to build without Connect.
ARG CONNECT_AUTH_URL=
ARG CONNECT_AUTH_PUBLISHABLE_KEY=
ENV CONNECT_AUTH_URL=${CONNECT_AUTH_URL}
ENV CONNECT_AUTH_PUBLISHABLE_KEY=${CONNECT_AUTH_PUBLISHABLE_KEY}

WORKDIR /app
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/frontend/package.json apps/frontend/package.json
COPY packages/addon-dev-tools/package.json packages/addon-dev-tools/package.json
COPY packages/addon-sdk/package.json packages/addon-sdk/package.json
COPY packages/ui/package.json packages/ui/package.json
ENV CI=1
ENV BUILD_TARGET=web
RUN --mount=type=cache,id=${CACHE_NAMESPACE}-pnpm-store,target=/pnpm/store,sharing=locked \
    corepack enable && pnpm config set store-dir /pnpm/store && pnpm install --frozen-lockfile
COPY . .
# Build only the main app to avoid building workspace addons in this image.
RUN pnpm --filter frontend... build && mv dist /web-dist

# Stage 2: build server with cross-compilation
FROM --platform=$BUILDPLATFORM tonistiigi/xx:latest@sha256:c64defb9ed5a91eacb37f96ccc3d4cd72521c4bd18d5442905b95e2226b0e707 AS xx

FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS backend-base
# Copy xx scripts to handle cross-compilation
COPY --from=xx / /
ARG TARGETPLATFORM

# Wealthfolio Connect configuration (baked into server binary at build time)
ARG CONNECT_AUTH_URL=
ARG CONNECT_AUTH_PUBLISHABLE_KEY=
ENV CONNECT_AUTH_URL=${CONNECT_AUTH_URL}
ENV CONNECT_AUTH_PUBLISHABLE_KEY=${CONNECT_AUTH_PUBLISHABLE_KEY}

WORKDIR /app

# Install build tools for the HOST (to run cargo, build scripts)
# clang/lld are needed for cross-linking
# pkgconfig is required for openssl-sys to find the target libraries
RUN apk add --no-cache clang21=21.1.2-r2 lld21=21.1.2-r1 git=2.52.0-r0 file=5.46-r2 pkgconf=2.5.1-r0

# Install TARGET dependencies
# xx-apk installs into /$(xx-info triple)/...
RUN xx-apk add --no-cache musl-dev=1.2.5-r23 gcc=15.2.0-r2 openssl-dev=3.5.9-r0 openssl-libs-static=3.5.9-r0 sqlite-dev=3.53.4-r0

# Install rust target
RUN rustup target add $(xx-cargo --print-target-triple)

# cargo-chef compiles third-party dependencies in their own layer, keyed on
# the manifests and lockfile only. The publisher builds on a fresh BuildKit
# builder every run, so a layer (restored from the registry cache) is the only
# place compiled dependencies survive; a cache mount would be discarded.
RUN cargo install cargo-chef --locked --version 0.1.78

ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV CARGO_BUILD_JOBS=4
ENV CARGO_PROFILE_RELEASE_DEBUG=0
ENV RUSTFLAGS="-C link-arg=-Wl,--no-keep-memory"
ENV OPENSSL_STATIC=1

# Reduce the workspace to a dependency recipe. This stage reruns on every
# source change, but its output only changes when a manifest or the lockfile does.
FROM backend-base AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/server ./apps/server
# Stub out apps/tauri so the workspace resolves (not built in Docker)
COPY apps/tauri/Cargo.toml apps/tauri/Cargo.toml
RUN mkdir -p apps/tauri/src && echo "fn main(){}" > apps/tauri/src/main.rs && echo "" > apps/tauri/src/lib.rs
RUN cargo chef prepare --recipe-path recipe.json

FROM backend-base AS backend
COPY --from=planner /app/recipe.json recipe.json
RUN xx-cargo chef cook --release --recipe-path recipe.json --package wealthfolio-server

# Now copy full sources; only the workspace crates are compiled here.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/server ./apps/server
COPY apps/tauri/Cargo.toml apps/tauri/Cargo.toml
RUN mkdir -p apps/tauri/src && echo "fn main(){}" > apps/tauri/src/main.rs && echo "" > apps/tauri/src/lib.rs
RUN xx-cargo build --release --manifest-path apps/server/Cargo.toml && \
    cp target/$(xx-cargo --print-target-triple)/release/wealthfolio-server /wealthfolio-server

# Final stage
FROM alpine:3.21@sha256:f27cad9117495d32d067133afff942cb2dc745dfe9163e949f6bfe8a6a245339
ARG SOURCE_REVISION
LABEL org.opencontainers.image.revision="${SOURCE_REVISION}" \
      org.opencontainers.image.source="https://git.example.com/andtoni/wealthfolio"
RUN apk upgrade --no-cache
WORKDIR /app
# Copy from backend (which is now build platform, but binary is target platform)
COPY --from=backend /wealthfolio-server /usr/local/bin/wealthfolio-server
COPY --from=frontend /web-dist ./dist
ENV WF_DB_PATH=/data/wealthfolio.db
# Wealthfolio Connect API URL (can be overridden at runtime via -e or docker-compose)
ARG CONNECT_API_URL=
ENV CONNECT_API_URL=${CONNECT_API_URL}

# Run as non-root. chown /data BEFORE the VOLUME directive so named volumes
# inherit ownership on first creation. Existing volumes from older images
# need a one-time chown — see docs/self-host/README.md.
RUN addgroup -S -g 1000 wealthfolio \
 && adduser -S -u 1000 -G wealthfolio -H -s /sbin/nologin wealthfolio \
 && mkdir -p /data \
 && chown -R wealthfolio:wealthfolio /data
USER 1000:1000

VOLUME ["/data"]
EXPOSE 8088
CMD ["/usr/local/bin/wealthfolio-server"]
