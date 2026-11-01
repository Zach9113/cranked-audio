# cranked-ipc

- Holds the contracts shared by the service and the window: messages, snapshots and the settings file format.
- No service or window logic. File and environment access is limited to the settings files and the socket path.
- Settings files are a contract with every installed copy:
  - Only the service writes `config.toml` and `state.toml`; the window and the widget change settings through the
    service.
  - A file written by any released version must keep loading: new fields take `#[serde(default)]`, a renamed field
    or value keeps its old spelling readable, and loading never silently discards a user's settings.
  - A change to the file's shape comes with a test in `src/settings.rs` that parses the old form.
