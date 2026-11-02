#!/usr/bin/env python3
"""Writes a mono 48 kHz 16-bit WAV for the live suites' test streams: silence, or the Auto-lower "talk" pattern.
WAV rather than raw samples, because pw-play only reads raw input with --raw, which older PipeWire (1.0) lacks.
Usage: make_wav.py OUT.wav SECONDS   (silence)
       make_wav.py OUT.wav talk      (0.5 s silence, a 0.3 s ping, 1.2 s silence, 1 s 440 Hz tone, 6 s silence)"""
import math
import sys
import wave

RATE = 48000


def silence(seconds):
    return bytes(int(RATE * seconds) * 2)


def tone(seconds):  # 440 Hz at about -12 dBFS
    return b"".join(
        int(8000 * math.sin(2 * math.pi * 440 * i / RATE)).to_bytes(2, "little", signed=True)
        for i in range(int(RATE * seconds))
    )


out, what = sys.argv[1], sys.argv[2]
frames = silence(0.5) + tone(0.3) + silence(1.2) + tone(1.0) + silence(6) if what == "talk" else silence(float(what))
with wave.open(out, "wb") as wav:
    wav.setnchannels(1)
    wav.setsampwidth(2)
    wav.setframerate(RATE)
    wav.writeframes(frames)
