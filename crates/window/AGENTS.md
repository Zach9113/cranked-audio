# cranked-audio (window)

- Talks to the service only through `cranked-ipc`; never opens the mixer or edits settings files directly.
- The command-line modes (`--knob`, `--press`, `--profile`, `--state`, `--watch`, `--waybar`) are clients of the
  service like the window: they send commands or read snapshots and keep no state of their own.
