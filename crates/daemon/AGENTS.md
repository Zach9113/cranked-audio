# cranked-audiod

- Runs as a systemd user service on Linux; it is the only writer of settings and state.
- New operating-system code goes inside `src/platform/`, and the rest of the service uses the ports. The `#[cfg]`
  blocks in `serial.rs` and `service.rs` move there only in a change made for that purpose.
- After a change that touches the mixer, audio routing or Home Assistant, run the matching live suite against a test
  copy, never against the installed service.
