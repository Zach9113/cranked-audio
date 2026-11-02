# Review: the window program (timeline step 2)

Step 2 of the timeline: *review and scrutinize the code, create better abstractions*. The service went through this
already (ports and adapters, `docs/platform-ports.md`). The window program (`crates/window`) didn't, and it shows:
almost everything is in one file. This is the review, and a plan in small steps, each one reviewable on its own.

## What's there

| File | Lines | What |
|---|---|---|
| `src/main.rs` | 2033 (82 of them tests) | everything below |
| `ui/settings.slint` | 1176 | the Settings, knob, device and LED pages |
| `ui/main.slint` | 676 | the knobs page and the shared widgets |
| `src/editor.rs` | 388 | lists of actions, amounts and swatches; app-list helpers |
| `src/themes.rs` | 200 | the colour themes |
| `src/waybar.rs` | 140 | `--waybar` output |

`main.rs` holds seven different jobs:

1. **Command line** (`--knob`, `--state`, `--watch`, `--waybar`, ...): never opens a window, needs no Slint.
2. **The link to the service**: the socket, reconnecting, and starting the service when it isn't running.
3. **Drawing helpers**: dial arcs, ticks, icons, colours, theme mixing.
4. **The knobs page**: a `Snapshot` turned into `KnobData` rows.
5. **The settings editor**: about 35 load/store functions between the `Config` and the Slint globals.
6. **The devices page**: keyboard knobs, MIDI learn, deej, and their hints.
7. **Wiring**: `setup_settings` alone is 480 lines and registers 34 callbacks.

## Findings

**1. One file, one 480-line function.** Changing a setting means finding the right closure among 34 in
`setup_settings`. The load/store functions for one page are spread across the file. This is the main thing to fix,
and it's mechanical: move code, don't change it.

**2. Platform code outside a platform layer.** The service keeps every OS difference behind a port. The window has
a few of its own, inline:

- starting the service: `systemctl --user start` on Linux, otherwise spawning `cranked-audiod` (`run_link`);
- opening a file: `xdg-open` or `explorer` (`open_with_default_app`);
- running `cranked-audiod --print-config` to find the settings file;
- wording: `cfg!(windows)` choosing between texts that mention the widget and Atoll or not (`get_device_hint`).

They're small, but they're the same kind of thing the service moved into adapters. Proposal: a tiny
`platform` module in the window with one trait, `Host` (`start_service`, `open_file`), and a Linux and a Windows
adapter, picked once in `platform::native()`, as in the service. The wording would come from what the platform has
(a `has_panel_widget` flag on the same adapter), not from `cfg!` in sentences.

**3. Logic that can't be tested because it's tied to Slint.** The load/store functions read and write Slint
globals directly, so "loading this config and saving it changes nothing" can't be a unit test. The fix is the
same as in the service's `Core`: plain structs for each page's values (`GeneralForm`, `KnobForm`), with
`Config -> form` and `form -> Config` as plain functions (testable), and a thin layer copying forms to and from the
globals. The round trip then gets a test per page.

**4. The command line lives in the GUI program.** `cranked-audio --knob 2 +5` loads a binary that links Slint. It
starts fast (2 ms on Linux, 11-20 ms on Windows), so this is about clarity, not speed. Moving it to `cli.rs` is
enough for now; a separate `cranked-audioctl` binary would only be worth it if packagers ask.

**5. Small things.**
- `Link` is shared as `Arc<Mutex<Link>>` with every callback, and each one locks it. Fine at this size; worth a
  note, not a change.
- `get_device_hint` and `populate_knob_device_choices` (100+ lines) build user-facing text in Rust; they belong
  with the devices page.
- The window has 15 unit tests; the service has 100+.

## Plan

Each step builds, passes clippy on both platforms, the tests and the live suites, and changes no behaviour.

1. **`cli.rs`**: the command line (`remote_line`, `send_remote`, `print_state`, `--waybar`) moves out of `main.rs`.
2. **`link.rs`**: `Link`, `run_link`, `set_status`.
3. **`dial.rs`**: arcs, ticks, icons, colours.
4. **One module per page**: `knobs_page.rs`, `settings_page.rs`, `knob_page.rs`, `devices_page.rs`, `leds_page.rs`.
   `setup_settings` becomes a call to each page's `setup`. After this, `main.rs` is wiring only (a few hundred
   lines).
5. **`platform` with the `Host` port** (finding 2), Linux and Windows adapters, and the wording flag.
6. **Forms** (finding 3), one page at a time, each with a round-trip test.

Steps 1-4 only move code, so they can go quickly; 5 and 6 are the real design changes and deserve a closer look.

## Questions for Thomas

- Is a `Host` port the right shape for the window's few OS differences, or would you rather the window ask the
  service (which already knows the platform) for anything OS-specific?
- Forms: plain structs per page, as above, or something else you prefer for testing the settings round trip?
- Should the command line stay in `cranked-audio`, or become its own small binary?
