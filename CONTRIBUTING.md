# Contributing to teams-cli

Thanks for your interest in contributing! This project aims to provide a secure, scriptable Microsoft Teams CLI for developers and AI agents.

- Code: Rust 2021, `clap` v4, async via `tokio`, HTTP via `reqwest`.
- Style: run `cargo fmt` and `cargo clippy -- -D warnings` before pushing.
- Tests: add unit tests near changed code; for HTTP, prefer `wiremock` for integration tests.
- Commits: conventional, clear messages. Small, focused PRs are easier to review.
- Security: never include secrets in tests, examples, or logs.

## Dev setup

```bash
rustup toolchain install stable
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

### Storage namespace

A debug build (`cargo build`, `cargo run`) stores its tokens under the keyring
service `teams-cli-dev` and reads its config from a `teams-cli-dev` directory
next to the usual `teams-cli` one. It therefore never reads or rewrites the
tokens and config of an installed release, and a source build does not raise
keychain prompts against the release's items on macOS. Sign in once with the
debug build. Release builds, including `cargo install`, keep `teams-cli`.

To choose the namespace, set `TEAMS_CLI_BUILD_NAMESPACE` when building. It is
read at compile time, not when the binary runs, and must be `teams-cli` or
`teams-cli-` followed by lowercase letters, digits and hyphens:

```bash
TEAMS_CLI_BUILD_NAMESPACE=teams-cli-dev cargo build --release
```

`teams --version` names the namespace of any build that does not use
`teams-cli`, and `teams config path` reports it in every build.

## Pull Requests

- Write a descriptive title and summary.
- Link related issues.
- Include usage notes and sample JSON if useful.
- Update docs/README where applicable.

## Code of Conduct

This project follows a standard Code of Conduct. Be respectful and inclusive.
