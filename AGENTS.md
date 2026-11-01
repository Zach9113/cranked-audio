# Repository instructions

- `nix develop` starts the pinned development environment (see the README); without Nix, a stable Rust toolchain and
  the libraries listed there are enough.
- Cranked Audio is per-app volume control: a service (`crates/daemon`, `cranked-audiod`, run as a systemd user
  service), a window and command line (`crates/window`, `cranked-audio`), the shared types (`crates/ipc`,
  `crates/protocol`) and a Plasma widget (`plasmoid/`). `SPEC.md` defines the intended behaviour.
- Make the smallest coherent change and avoid speculative dependencies or abstractions.
- Don't add a crate, a binary, a persisted file, a settings key, an IPC message, a D-Bus method or a command-line
  flag unless the task asks for it; describe the proposal instead.
- Change only the code the task needs. Leave unrelated restructuring, renames and lint clean-ups for their own
  change.
- Don't change what the app does for its users without the maintainers agreeing first: a fix restores the intended
  behaviour, a change in behaviour is a decision.
- Use tool-neutral language for development workflows in documentation and source comments; avoid references to
  paid or enterprise products.
- Describe durable behaviour and contracts in documentation and comments. Avoid time-relative status wording such as
  "currently", "for now", "initial implementation" or "not yet", and avoid narrating the editing session. Keep
  progress and release status in dated records (`CHANGELOG.md`); never imply that unimplemented behaviour exists.
- Operating-system code in the service goes behind the ports in `crates/daemon/src/platform/`
  (`docs/platform-ports.md`): a trait, one adapter per platform as a `Box<dyn …>`, picked once in
  `platform::native()`. `unsafe` code is allowed only in the platform adapters.
- Put implementations in descriptively named module files. New directory modules keep `mod.rs` to declarations and
  re-exports; existing `mod.rs` files that hold shared code move only in a change made for that purpose.
- Use explicit names and keep the workspace lints clean (`clippy -D warnings`); don't silence a lint without a
  comment saying why.
- Code, structure and wording are this project's own; don't copy them from other volume-control applications.
- Settings files written by any released version must keep loading; the rules are in `crates/ipc/AGENTS.md`.
- The IPC types in `crates/ipc` and the D-Bus API are shared by the service, the window, the widget and users'
  scripts; changing them is a behaviour change.
- Never write the Home Assistant token to logs, errors, `Debug` output or test fixtures. The updater installs only a
  release whose `.sha256` matches.
- Follow crate-local `AGENTS.md` files for boundary-specific rules.
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

At handoff, explain the change for the maintainer who will own it: what changed and why, the alternative that was
rejected, and the file and function to start reading from.
