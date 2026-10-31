# Testing every way people install Cranked Audio

Step 3 of the timeline: *stand up robust testing environments across distribution channels*. Today a release is
checked on one machine (Zach's KDE desktop), plus by hand in the VMs when someone remembers. CI only builds, lints
and runs unit tests on Ubuntu. Nothing checks that each thing a user downloads actually installs and runs on a
clean system. This plan makes that automatic, so a release is only published once every channel has passed.

## The channels

| Channel | Built by | Who uses it | Tested today |
|---|---|---|---|
| Source (`./install.sh` from git) | the user | packagers, contributors | Zach's PC, the VMs by hand |
| Release tarball (`install-release.sh`, the tarball's `install.sh`) | release CI, Ubuntu 24.04 | most Linux users | once, by hand (2.3.0 in the Fedora VM) |
| AppImage | release CI | people who don't want to install | never |
| AUR `cranked-audio-bin` | `packaging/aur`, repackages the tarball | Arch, CachyOS, EndeavourOS | `makepkg` once, by hand |
| Nix | `flake.nix` (today a dev shell only, no package) | Thomas, NixOS users | the dev shell builds |
| Windows zip (`install.ps1`) | release CI, Windows | Windows preview users | in the Windows VM, by hand |

One gap found while writing this: the release tarball is built on `ubuntu-latest` (glibc 2.39), so its programs
won't start on anything older (Debian 12, Ubuntu 22.04, Mint 21). The tests below would show it; the fix is a
decision (see the end).

## Three tiers

**Tier 1, every push (CI).** Fast checks that catch most regressions before a release exists:
- what CI does today (fmt, clippy on Linux and Windows, unit tests), plus
- **the six live suites** (`tests/live*.sh`) on the Ubuntu runner, with PipeWire and a session bus started headless
  in the job. Today they only run on Zach's PC.
- **a source build on Arch and Fedora** in containers, so a newer compiler or a missing `-devel` package shows up.

**Tier 2, every release, before it's published (CI).** The release workflow uploads to a *draft* release. New jobs
then install each channel on clean containers and runners, and the release is published only when all pass:

| Channel | Where | Check |
|---|---|---|
| Tarball | containers: Arch, Fedora, Ubuntu 24.04, Debian 13, openSUSE Tumbleweed (plus the older ones once the glibc floor is decided) | checksum matches; both programs start (`--version`); `install.sh` into a throwaway home puts every file where the README says; the service starts against a headless PipeWire and answers on its socket and on D-Bus; `--uninstall` removes everything and keeps the settings |
| AppImage | the same containers | runs with `--appimage-extract-and-run` (no FUSE in containers); the service answers on its socket |
| AUR | `archlinux:base-devel` container | the PKGBUILD pointed at the draft's tarball: `makepkg -si`, `namcap`, files land in `/usr`, the in-app updater says to update with the package manager |
| Nix | `nix build` on the Ubuntu runner | needs a package output in the flake first (see the end); then `nix build` and `nix flake check` |
| Windows zip | `windows-latest` runner | signature check once signing is on; `install.ps1` installs, the service starts and answers, `install.ps1 -Uninstall` removes it. The runner has no sound device, so the volume suites stay in the VM |

**Tier 3, before a release, on real desktops (by script, run by hand).** What containers can't show: trays, lock
screens, shortcut dialogs, a real mixer.
- `tests/vm/linux.sh <draft-tag>`: boots the Fedora VM, installs the draft tarball, logs into GNOME, Hyprland and
  Sway in turn, and checks the tray, lock detection and "Set shortcuts…" in each (what this week's VM rounds did by
  hand). Shuts the VM down.
- `tests/vm/windows.sh <draft-tag>`: the same in the Windows VM with `tests/live_windows*.ps1`.
- Zach's PC (KDE, the real mixer): install the draft, the live suites, a look at the lights.

A release checklist (`docs/releasing.md`) lists the three tiers in order, so whoever cuts a release runs the same
steps.

## Running it locally

`scripts/test-channel.sh <channel> [<tag>|local]` runs any tier 2 check in Docker on your own machine, against a
draft release or a tarball built from the working tree. CI calls the same script, so a CI failure can be reproduced
with one command.

## Order of work

Each step is small enough to review on its own:

1. **Live suites in CI** (tier 1). Biggest payoff: every push runs what only Zach's PC runs today.
2. **`scripts/test-channel.sh` with the tarball and AppImage containers**, run by hand first.
3. **Draft-then-publish release flow**: the release job uploads a draft, the channel jobs test it, and the release
   is published only when all pass.
4. **AUR** container test.
5. **Windows zip** runner test.
6. **Source builds on Arch and Fedora** (tier 1).
7. **VM scripts** and `docs/releasing.md` (tier 3).
8. **Nix package**, with Thomas (see below).

## Decisions needed

- **glibc floor (Zach and Thomas).** Build the tarball and AppImage on Ubuntu 22.04 (glibc 2.35: also runs on
  Debian 12, Mint 21 and older Fedora) instead of the newest Ubuntu? Recommended: yes. It costs nothing at runtime
  and widens who can run the prebuilt release.
- **Nix (Thomas).** Add a `packages.default` to the flake so `nix build` and `nix run` work and CI can test it, or
  leave Nix users to nixpkgs? Thomas owns the Nix side, so it's his call.
- **Does a failing channel block the release?** Recommended: yes. The draft stays a draft until it's fixed, or
  until someone decides to publish anyway.
