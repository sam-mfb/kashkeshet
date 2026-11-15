# Rust

This document says what [ARCHITECTURE.md](ARCHITECTURE.md) and [CODING.md](CODING.md) mean for a project written in Rust. A module from those documents is a crate.

1. A module is a crate. The repository is a Cargo workspace with one crate per module.

2. Every dependency's version and features are declared once, at the workspace level, and each crate takes what it uses from there.

3. The toolchain is pinned in `rust-toolchain.toml`, and builds use the lockfile.

4. Visibility is the module boundary. An item is `pub` only when another crate needs it.

5. A loosely coupled dependency is a trait, declared by the consuming crate (adapter model) or by the crate that provides the contract (shared contract model), with an implementation supplied at startup. A crate may depend directly on the public items of a logic crate.

6. Stand-ins and other test support are behind a `testing` feature that only test code turns on. A stand-in may be built with any tool, including a library the real implementation uses, but shares no code with the real implementation.

7. A third-party crate is imported and wrapped by one or more project crates, and its types stay inside those crates. The number of wrapping crates should be minimized, but is ultimately up to the developer's judgment. This rule does not apply to the third-party crates that shape the language, though rules about side effects still apply. Those crates are: thiserror, futures, serde, serde_json, tokio, tokio-util. A human MUST approve any addition to this list.

8. Errors are typed with `thiserror`. Program code returns `Result`; `unwrap` and `expect` are for tests.

9. A lint exception uses `#[expect(..., reason = "...")]`, never `#[allow]`.

10. `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` MUST be clean before every commit and in CI.

11. Unit tests sit beside the code they test. Integration tests live in `tests/`. A test of real components together is a named suite of its own that says what it is for, not an exception made inside another crate's tests.

12. A crate may mix kinds in two cases. In both, the kinds are kept apart by a `real` feature that only programs turn on.

12.1 A contract and its only implementation may share a crate. The implementation is behind `real`.

12.2 A logic crate may hold the effect that carries out its plans. The effect is behind `real`.
