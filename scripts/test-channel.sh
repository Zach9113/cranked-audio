#!/usr/bin/env bash
# Tests a release's Linux downloads (tarball and AppImage) on clean systems, one container each: installs only what
# a desktop already has (PipeWire, D-Bus, fonts), then runs tests/channel/release_check.sh as a normal user in a
# headless session. CI runs it on every release before it's published; it works the same on your machine.
# Usage: scripts/test-channel.sh DIR_WITH_RELEASE_FILES [IMAGE...]
#        scripts/test-channel.sh --release v2.4.0 [IMAGE...]   (downloads that release's files first)
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
if [ "${1:-}" = "--release" ]; then
  assets=$(mktemp -d)
  chmod 755 "$assets" # the containers' test user isn't you
  gh release download "${2:?a tag}" -R Zach9113/cranked-audio -D "$assets" -p '*x86_64.tar.gz*' -p '*.AppImage'
  shift 2
else
  assets=$(cd "${1:?usage: test-channel.sh DIR|--release TAG [IMAGE...]}" && pwd)
  shift
fi
images=("$@")
[ ${#images[@]} -gt 0 ] || images=(ubuntu:22.04 ubuntu:24.04 debian:12 debian:13 fedora:latest archlinux:latest
  opensuse/tumbleweed:latest)

# what a desktop already has, per package manager
setup() {
  case "$1" in
  ubuntu:* | debian:*) echo 'export DEBIAN_FRONTEND=noninteractive; apt-get update -qq && apt-get install -y -qq pipewire pipewire-pulse wireplumber pulseaudio-utils dbus dbus-user-session fontconfig libxcb1 python3 passwd' ;;
  fedora:*) echo 'dnf install -y -q pipewire pipewire-pulseaudio wireplumber pulseaudio-utils /usr/bin/dbus-run-session fontconfig libxcb alsa-lib python3 shadow-utils util-linux' ;;
  archlinux:*) echo 'pacman -Syu --noconfirm --needed pipewire pipewire-pulse wireplumber libpulse dbus fontconfig libxcb alsa-lib python' ;;
  opensuse/*) echo 'zypper -n -q in pipewire pipewire-pulseaudio wireplumber pulseaudio-utils /usr/bin/dbus-run-session fontconfig libxcb1 alsa python3 shadow util-linux' ;;
  *) echo "no package list for $1" >&2; return 1 ;;
  esac
}

failed=()
for image in "${images[@]}"; do
  echo "=== $image"
  if docker run --rm --network host -v "$root/tests:/tests:ro" -v "$assets:/assets:ro" "$image" sh -c "
      ( $(setup "$image") ) > /tmp/setup.log 2>&1 || { tail -20 /tmp/setup.log; exit 1; }
      useradd -m tester && su tester -c '/tests/headless_session.sh /tests/channel/release_check.sh /assets'"; then
    echo "=== $image: passed"
  else
    echo "=== $image: FAILED"
    failed+=("$image")
  fi
done
if [ ${#failed[@]} -gt 0 ]; then
  echo "failed on: ${failed[*]}"
  exit 1
fi
echo "every system passed"
