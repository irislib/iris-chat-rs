#!/usr/bin/env python3
"""Generate Iris's original, gently faded outgoing call tones (mono PCM WAV)."""
import math
from pathlib import Path
import struct
import wave

RATE = 16000
ROOT = Path(__file__).resolve().parents[1] / "core" / "assets" / "call-audio"
ROOT.mkdir(parents=True, exist_ok=True)
for name, duration, pulses, frequencies in [
    ("connecting", 2, [(0, .12), (.3, .42)], (440,)),
    ("ringing", 6, [(0, 2)], (440, 480)),
]:
    samples = []
    for index in range(RATE * duration):
        time = index / RATE
        envelope = max((max(0, min(1, (time - start) / .01, (end - time) / .01))
                        for start, end in pulses), default=0)
        value = sum(math.sin(2 * math.pi * hz * time) for hz in frequencies) / len(frequencies)
        samples.append(round(32767 * .18 * envelope * value))
    with wave.open(str(ROOT / f"call-{name}.wav"), "wb") as audio:
        audio.setparams((1, 2, RATE, 0, "NONE", "not compressed"))
        audio.writeframes(struct.pack(f"<{len(samples)}h", *samples))
