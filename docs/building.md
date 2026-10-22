# Building and releases

Needs Rust 1.88+.

```sh
cargo build --release   # → target/release/monadar
cargo test              # unit tests for parsing, frame math, metrics, OTLP, service generation
cargo clippy --all-targets
```

`src/fixture_replay.rs` replays bft/exec journals and raw OTLP exports captured on a mainnet
validator and checks the results against reference numbers (`expected.json`, computed by the
former TypeScript implementation). The fixtures (~100 MB) are not in the repo; point the test at
an unpacked fixture directory:

```sh
MONADAR_FIXTURES=/path/to/2026-10-22 cargo test --release fixture -- --nocapture
```

For the node (x86_64 Linux) build a static musl binary. The script runs the build in an
x86_64 Alpine container, so it works from macOS too (needs Docker). It uses the baseline
x86-64 CPU target, not `target-cpu=native`, so the binary runs on any x86_64 host:

```sh
./scripts/build-musl.sh  # → target/x86_64-unknown-linux-musl/release/monadar
```

The binary is self-contained: copy it to the node (see [Install](../README.md#install)).

## Releases and verification

Pushing a tag `v*` runs `.github/workflows/release.yml`. The workflow tests, builds with the
same `scripts/build-musl.sh`, and publishes `monadar-<tag>-x86_64-linux-musl` with `SHA256SUMS`
as a GitHub release.

The build is reproducible. The toolchain is a single container image pinned by digest (rustc,
musl, linker), crates are pinned by `Cargo.lock`, and paths inside the container are fixed. So
anyone can check that a release binary was built from the tagged source:

```sh
sha256sum -c SHA256SUMS                         # the download is intact
git checkout v1.0.0 && ./scripts/build-musl.sh  # prints the same sha256 as SHA256SUMS
```

The workflow also signs a build provenance attestation: a statement from GitHub that this file
was built by this workflow from this repository at this commit. `gh attestation verify <file>
-R coinblitz-pro/monadar` checks it, no rebuild needed.

`monadar --version` prints the build: `git describe` of the source, which is the tag for a
release. The viewer uses it to tell when the daemon runs a different build.
