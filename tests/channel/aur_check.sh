#!/usr/bin/env bash
# Builds and installs the AUR package (packaging/aur/cranked-audio-bin) from a release's tarball on a clean Arch,
# as an AUR helper would, then checks what it installed and that it runs. scripts/test-channel.sh runs it as root
# in an archlinux:base-devel container, with the repository at /repo and the release files at /assets.
set -u
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($2)"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails + 1)); fi; }

tarball=$(ls /assets/cranked-audio-v*-x86_64.tar.gz 2>/dev/null | head -1)
[ -n "$tarball" ] || { echo "FAIL no tarball in /assets"; echo "failures: 1"; exit 1; }
version=$(basename "$tarball" | sed 's/cranked-audio-v\(.*\)-x86_64.tar.gz/\1/')

pacman -Syu --noconfirm --needed namcap pacman-contrib pipewire pipewire-pulse wireplumber python > /tmp/setup.log 2>&1 ||
  { tail -20 /tmp/setup.log; exit 1; }
useradd -m builder
# makepkg installs the dependencies through sudo (this container is thrown away afterwards)
echo 'builder ALL=(ALL) NOPASSWD: /usr/bin/pacman' > /etc/sudoers.d/builder
build=/home/builder/build
mkdir -p "$build"
cp /repo/packaging/aur/cranked-audio-bin/* "$build/"
# the release's files stand in for the downloads (makepkg uses a source that's already there)
cp "$tarball" "$build/"
cp /repo/LICENSE "$build/LICENSE-$version"
sed -i "s/^pkgver=.*/pkgver=$version/" "$build/PKGBUILD"
chown -R builder "$build"
su builder -c "cd $build && updpkgsums >/dev/null 2>&1 && makepkg --syncdeps --noconfirm --nocheck >/tmp/makepkg.log 2>&1" ||
  { tail -30 /tmp/makepkg.log; }
package=$(ls "$build"/cranked-audio-bin-"$version"-*.pkg.tar.zst 2>/dev/null | head -1)
check "makepkg builds the package" "$([ -n "$package" ] && echo yes)" "yes"
[ -n "$package" ] || { echo "failures: $fails"; exit "$fails"; }
echo "--- namcap:"; namcap "$build/PKGBUILD" "$package" || true
pacman -U --noconfirm "$package" > /tmp/install.log 2>&1
check "pacman installs it" "$?" "0"
for f in /usr/bin/cranked-audiod /usr/bin/cranked-audio /usr/lib/systemd/user/cranked-audio.service \
  /usr/share/applications/io.github.zach9113.CrankedAudio.desktop /etc/xdg/autostart/io.github.zach9113.CrankedAudio.Autostart.desktop \
  /usr/lib/udev/rules.d/60-cranked-audio.rules /usr/share/plasma/plasmoids/io.github.zach9113.crankedaudio/metadata.json; do
  check "installed $f" "$([ -e "$f" ] && echo yes || echo no)" "yes"
done
# runs as a normal user, from /usr/bin
useradd -m tester
su tester -c "/repo/tests/headless_session.sh sh -c '
  export CRANKED_SOCKET=/tmp/aur-socket XDG_STATE_HOME=/tmp/aur-state
  printf \"popups = false\n[[channel]]\nname = \\\"T\\\"\n\" > /tmp/aur.toml
  cranked-audiod --config /tmp/aur.toml --no-mixer > /tmp/aur.log 2>&1 &
  sleep 2; cranked-audio --knob 1 30; sleep 0.5
  cranked-audio --state > /tmp/aur-state.json'"
check "the installed programs run" "$(python3 -c "import json; print(json.load(open('/tmp/aur-state.json'))['knobs'][0]['level'])" 2>/dev/null)" "30"
pacman -R --noconfirm cranked-audio-bin > /dev/null 2>&1
check "pacman removes it" "$([ -e /usr/bin/cranked-audiod ] && echo no || echo yes)" "yes"
echo "failures: $fails"
exit $fails
