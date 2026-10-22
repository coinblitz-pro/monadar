#!/bin/sh
# Static Linux build for the node: target/x86_64-unknown-linux-musl/release/monadar.
# Runs in an x86_64 Alpine container, so it works from macOS too (needs Docker).
# No target-cpu=native: the binary must run on any x86_64 CPU.
set -e
cd "$(dirname "$0")/.."
docker run --rm --platform linux/amd64 \
  -v "$PWD":/src -w /src \
  -v monadar-cargo-registry:/usr/local/cargo/registry \
  -e MONADAR_GIT_HASH="$(git rev-parse --short HEAD 2>/dev/null || echo dev)" \
  rust:1.99-alpine \
  sh -c 'apk add --no-cache musl-dev >/dev/null && cargo build --release --locked --target x86_64-unknown-linux-musl'
ls -l target/x86_64-unknown-linux-musl/release/monadar
