# Repository instructions

- `nix develop` starts the pinned development environment (see the README); without Nix, a stable Rust toolchain and
  the libraries listed there are enough.
- Cranked Audio is per-app volume control: a service (`crates/daemon`, `cranked-audiod`), a window and command line
  (`crates/window`, `cranked-audio`), the shared types (`crates/ipc`, `crates/protocol`) and a Plasma widget
  (`plasmoid/`).
- Make the smallest coherent change and avoid speculative dependencies or abstractions.
- Don't change what the app does for its users without the maintainers agreeing first: a fix restores the intended
  behaviour, a change in behaviour is a decision.
- Use tool-neutral language for development workflows in documentation and source comments; avoid references to
  paid or enterprise products.
- Describe durable behaviour and contracts in documentation and comments. Avoid time-relative status wording such as
  "currently", "for now", "initial implementation" or "not yet", and avoid narrating the editing session. Keep
  progress and release status in dated records (`CHANGELOG.md`); never imply that unimplemented behaviour exists.
- Every operating-system difference in the service lives behind a port in `crates/daemon/src/platform/`
  (`docs/platform-ports.md`): a trait, one adapter per platform as a `Box<dyn …>`, picked once in
  `platform::native()`. `unsafe` code is allowed only in the platform adapters.
- Use directory modules with `mod.rs` for declarations and re-exports only, and put implementations in descriptively
  named module files.
- Use explicit names and keep the workspace lints clean (`clippy -D warnings`); don't silence a lint without a
  comment saying why.
- Code, structure and wording are this project's own; don't copy them from other volume-control applications.
- Changes to the live behaviour need a check in the matching live suite (`tests/live*.sh`, `tests/live_windows*.ps1`).
- Test copies of the service run with their own socket and settings (`CRANKED_SOCKET`, `XDG_STATE_HOME`,
  `--config`, `--no-mixer`) and are stopped afterwards, so they never fight a running installation over the mixer or
  the audio.

During development, run the smallest checks relevant to the change.

Before declaring work merge-ready, run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings   # cargo xwin on Linux
cargo test --workspace
for suite in tests/live*.sh; do "$suite"; done   # in a desktop session with PipeWire
```

If the full set of checks was not run, state that clearly at handoff.
