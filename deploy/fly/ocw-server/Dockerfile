# syntax=docker/dockerfile:1.7
# The game server with its bundled voice server for Fly.io (README "Hosting
# the server on Fly.io", spec 11.5). Built from the repository root:
#
#   fly deploy --config deploy/fly/ocw-server/fly.toml \
#     --dockerfile deploy/fly/ocw-server/Dockerfile \
#     --ignorefile deploy/fly/ocw-server/Dockerfile.dockerignore --remote-only .
#
# Stage 1 builds only `ocw-server` (release, default features) with the
# Rust of rust-toolchain.toml; stage 2 is a small Debian with the binary,
# the pinned livekit-server and the start script, run as a user of its own.

# Rust as in rust-toolchain.toml (the file itself stays out of the build
# context, so rustup does not fetch its extra components and targets).
FROM rust:1.99.1-slim-trixie AS build
WORKDIR /src
COPY . .
# The models are Git LFS files: a checkout without them has pointer files.
RUN head -c 4 artwork/3dmodels/riders/rider-body.glb | grep -q glTF \
 || { echo "artwork/3dmodels holds LFS pointers: run git lfs pull before deploying" >&2; exit 1; }
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p ocw-server --bin ocw-server \
 && install -m 0755 target/release/ocw-server /usr/local/bin/ocw-server

# The voice server (spec 7.1.1): the release marge's tests use, checked
# against the release's published checksum.
FROM debian:trixie-20261104-slim AS livekit
ARG LIVEKIT_VERSION=1.13.10
ARG LIVEKIT_SHA256=c25a3e45fb922e9a93c6443d10c0ed041ba64342097e38632331736630283165
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSLo /tmp/lk.tgz "https://github.com/livekit/livekit/releases/download/v${LIVEKIT_VERSION}/livekit_${LIVEKIT_VERSION}_linux_amd64.tar.gz" \
 && echo "${LIVEKIT_SHA256}  /tmp/lk.tgz" | sha256sum -c - \
 && tar -xzf /tmp/lk.tgz -C /usr/local/bin livekit-server \
 && chmod 0755 /usr/local/bin/livekit-server

FROM debian:trixie-20261104-slim
# ca-certificates: the real weather's HTTPS requests (spec 14 Phase 9 S7).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --user-group --home-dir /data/home --no-create-home ocw
COPY --from=build /usr/local/bin/ocw-server /usr/local/bin/ocw-server
# Next to ocw-server: the server finds it there (`--voice-server` default).
COPY --from=livekit /usr/local/bin/livekit-server /usr/local/bin/livekit-server
COPY deploy/fly/ocw-server/start.sh /usr/local/bin/ocw-start
RUN chmod 0755 /usr/local/bin/ocw-start
# The start script prepares the volume as root, then runs the server as
# `ocw` (setpriv); nothing of the server runs as root.
ENTRYPOINT ["/usr/local/bin/ocw-start"]
