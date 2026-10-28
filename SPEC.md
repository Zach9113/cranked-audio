# Cranked Audio: behaviour spec

What the app must do, written from our own measurements and use. Implementation is free to differ.
Numbers marked (measured) come from probing the real mixer or the live system in October 2026.

## 1. Mixer hardware

- USB serial (CH343 chip, shows up as `/dev/serial/by-id/usb-1a86_USB_*` or USB vendor `1a86`), 115200 baud.
  Find it by that by-id pattern or vendor ID; `ttyACMn` numbering is unreliable because monitors with USB hubs
  also create ttyACM devices. Open it exclusively.
- Opening the port resets the board (measured). For about 3 s after opening, the board reports every knob ramping
  from 0 up to its real position, then it only reports **changes**.
- Incoming packets all start with `FF FE`, then a type byte:
  - `03 ch hi lo`: knob `ch` (0-4) moved, value `hi<<8 | lo` in 0..1023.
  - `06 ch`: button `ch` pressed down. `07 ch`: button `ch` released.
  - `02`: heartbeat, about twice a second; ignore.
  - Anything else: skip two bytes and resync on the next `FF FE`. Values above 1023 are corrupt (seen in the wild).
- LED frame (outgoing, 48 bytes): `FE 05`, then for knobs 0..4, three LEDs each (left, middle, right), each as
  R G B bytes, then `FF`. The board blanks the LEDs shortly after a frame (measured), so resend the current
  frame every 250 ms; send changes right away, but at most 25 frames per second.

## 2. Knob readings

- The pots never quite reach their ends, so 0..100% is spread over raw 20..1000 (readings below/above clamp):
  both ends are always reachable and every 1% step exists on the way (no jump from 3% to 0%).
- Drop readings above 1023.
- A jump of 25 points or more from the current value is held until the next reading for that knob confirms it
  (within 3 points). A lone spike is ignored. (These spikes caused full-volume blips.)
- 1% resolution, but a new value only counts once the raw reading has moved at least 7 (of 1023) since the last
  accepted value, so a knob resting on a boundary can't flicker. 0% and 100% always count.
- Startup sync: after opening the port, collect readings without acting on them until all five knobs have reported
  and nothing has arrived for 0.3 s (or 2.5 s have passed), then apply the final positions once. Never push the
  last session's saved positions before the knobs are read.
- Coalesce knob changes: apply at most every 50 ms, newest value wins.

## 3. Channels and audio

- Each knob is a channel with a name, a list of target apps (or a Home Assistant light, see 7), a volume range
  (min..max that the knob's 0..100 maps onto) and LED colours.
- Only **playback** streams are ever touched: never microphone capture, screen-share capture or duplex filter
  nodes, never the default source.
- Matching: a stream belongs to the target whose name/aliases match it best. "Everything else" (catch-all) takes
  every playback stream not matched by another channel, except system sounds (libcanberra, KDE, portals),
  voice chat (WebRTC VoiceEngine, Steam Voice, Vivox, Discord/Vesktop/Vencord, TeamSpeak, Mumble,
  `media.role` phone/communication) and ignored apps (NVIDIA Broadcast, the app's own test streams).
- New streams get their channel's volume as soon as they appear (event-driven, not polled).
- Fast guard: when a stream on a channel changes volume and it is more than 1% off the channel's volume, put it back
  immediately (Spotify rewrites its own volume on every seek/track change: measured 80% -> 51% for ~0.5 s).
  Our own writes already match, so they cause nothing. Never re-apply volumes on every change event: that makes a
  feedback loop that can keep WirePlumber at a third of a CPU core.
- Mute per channel (button action): mutes current and future streams of the channel.

## 4. Buttons

- A press is down + up within 0.5 s; holding 0.5 s or more fires the hold action immediately and skips the press.
  Ignore repeat presses within 0.2 s.
- Actions (separate choice for press and hold): none, mute channel, play/pause, next, previous (MPRIS, most
  recently active player), switch audio output (cycle real outputs, skip virtual ones like NVIDIA Broadcast; show
  the device name), Home Assistant toggle / turn off (an entity), next profile.

## 5. Feedback

- On-screen volume popup when a knob turns (channel or playing app name + level); a text popup for button
  actions (muted/unmuted, output name, profile name). Uses KDE's OSD when on Plasma, falls back to standard
  freedesktop notifications on GNOME, Sway, Hyprland, etc. Optional.
- LEDs per knob: fill from the left with the active colour by level, the rest in the background colour. Brightness
  and colour are worked out as eyes see them and turned into LED power last: dimming follows the CIE 1931 lightness
  curve (looking half as bright = about 18% power), and the LED the level is in mixes background to level colour in
  OKLab (the middle looks halfway, no grey or dark dip). The shown level glides to a new value (~70 ms time constant) instead of stepping. Muted channel = muted colour. Global brightness. Dim to look half as bright
  when the channel has no playing streams (optional). All off while the screen is locked or the displays are asleep;
  back on with the screen or any knob/button input.

## 6. Auto-lower (ducking)

- Settings: on/off, channels to lower, trigger channels, "anyone talking in voice chat", amount (off/20/30/50/70%),
  dim lowered channels' LEDs.
- Trigger = playback level of a trigger channel's streams or of voice-chat streams above about -44 dBFS for 50 ms.
  Measure the stream's own output (never the mic). A lowered channel never triggers itself.
- Fade down over 0.25 s, stay down until 1.2 s after the last trigger, fade back over 0.8 s. Knob turns, new
  streams and the fast guard all respect the lowered level. LEDs of lowered channels dim to look `factor` as bright (at least 35%).

## 7. Home Assistant (optional)

- Address + long-lived token (stored readable by the user only). Off until set up.
- A knob can control a light's brightness instead of audio (0% = off). If the light was changed elsewhere, the knob
  catches up proportionally instead of jumping: moving up, light' = light + (v - prev) * (100 - light) / (100 - prev);
  moving down, light' = light - (prev - v) * light / prev; within 3% they move 1:1.
- Live state from Home Assistant's event stream (`/api/stream`, state_changed), with a slow poll as fallback. Send light changes
  at most every 0.15 s, newest wins.
- Button actions can toggle/turn off a switch, light, fan, input_boolean, or run a scene/script. Show an instant
  "Turning on ..." popup, then the result.

## 8. Profiles

- A profile = the whole layout: per-channel apps, names, colours, ranges, lights, button actions, Auto-lower.
  Global settings (brightness, popups, Home Assistant connection) are shared. The live settings are the active profile.
- Switching applies the current knob positions to the new layout immediately and shows a popup.

## 9. On-screen controls / no mixer

- The window shows five knobs and five buttons with live levels, LED colours, the current source per channel and
  each button's press/hold actions.
- Optional on-screen control: drag or scroll a knob (Shift = 5%), click a button to press it, hold for the hold
  action. These produce the same input events as the hardware, so all rules above apply.
- Without a mixer (and on-screen control on), run entirely from the window; pick up a mixer that appears later.
  If the mixer exists but another program holds it, don't retry in a loop.

## 10. Process and resources

- One copy at a time (lock file). Auto-start at login, minimised to the tray.
- Idle cost targets: daemon < 15 MB RAM and ~0% CPU when nothing changes; no polling faster than once a second
  except while a fade or a held button is in progress.
