# Rust

This document says what [ARCHITECTURE.md](ARCHITECTURE.md) and [CODING.md](CODING.md) mean for a project written in Rust. A module from those documents is a crate.

1. A module is a crate. The repository is a Cargo workspace with one crate per module.

2. Every dependency's version and features are declared once, at the workspace level, and each crate takes what it uses from there.

3. The toolchain is pinned in `rust-toolchain.toml`, and builds use the lockfile.

4. Visibility is the module boundary. An item is `pub` only when another crate needs it.

5. A dependency is a trait, declared by the consuming crate or by a contract crate, with an implementation supplied at startup.

6. Stand-ins and other test support are behind a `testing` feature that only test code turns on.

7. A third-party crate is imported and wrapped by one or more project crates, and its types stay inside those crates. The number of wrapping crates should be minimized, but is ultimately up to the developer's judgment. This rule does not apply to third-party crates that shape the language (e.g., thiserror, futures, serde, tokio), though rules about side effects still apply.

8. Errors are typed with `thiserror`. Program code returns `Result`; `unwrap` and `expect` are for tests.

9. A lint exception uses `#[expect(..., reason = "...")]`, never `#[allow]`.

10. Logging uses `tracing` and happens only in side-effect and composition crates.

11. `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` MUST be clean before every commit and in CI.

12. Unit tests sit beside the code they test. Integration tests live in `tests/`.
