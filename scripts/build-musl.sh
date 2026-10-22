#!/bin/sh
# Reproducible static Linux build: target/x86_64-unknown-linux-musl/release/monadar.
#   ./scripts/build-musl.sh        build (prints the sha256)
#   ./scripts/build-musl.sh test   run the tests in the same environment
#
# The whole toolchain is one container image pinned by digest (rustc, musl,
# linker), crates are pinned by Cargo.lock, and paths inside the container are
# fixed (/src, /usr/local/cargo). The same commit therefore gives the same
# bytes here, in CI, and on anyone's machine: rebuild a release tag and
# compare the sha256 with the published one. Works from macOS too (Docker,
# x86_64 emulation). No target-cpu=native: the binary runs on any x86_64 CPU.
set -e
# rust:1.99-alpine, linux/amd64: rustc 1.99.1, musl 1.2.6, gcc 15.2.0
IMAGE=rust@sha256:e15c563b25397d4172f34ebe50b3715bb9509c59e537ff58d48902571387b881
cd "$(dirname "$0")/.."

case "${1:-build}" in
  build) cmd="cargo build --release --locked --target x86_64-unknown-linux-musl" ;;
  test) cmd="cargo test --locked" ;;
  *) echo "usage: $0 [build|test]" >&2; exit 2 ;;
esac

docker run --rm --platform linux/amd64 \
  -v "$PWD":/src -w /src \
  -v monadar-cargo-registry:/usr/local/cargo/registry \
  -e MONADAR_GIT_HASH="$(git describe --tags --always --dirty --abbrev=10 2>/dev/null || echo dev)" \
  "$IMAGE" $cmd

if [ "${1:-build}" = build ]; then
  bin=target/x86_64-unknown-linux-musl/release/monadar
  if command -v sha256sum >/dev/null; then sha256sum "$bin"; else shasum -a 256 "$bin"; fi
fi
