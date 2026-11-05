"""Key and energy: notation, the comment rewrite, and behaviour on known audio."""

from __future__ import annotations

import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np
import pytest

from vibecheck import keyenergy, rekordbox
from vibecheck.store import Label

SR = keyenergy.PRE.sample_rate


# --- notation ----------------------------------------------------------------

@pytest.mark.parametrize("tonic, minor, expected", [
    (9, True, "8A"),    # A minor
    (0, False, "8B"),   # C major, its relative
    (8, True, "1A"),    # G# minor
    (11, False, "1B"),  # B major
    (1, True, "12A"),   # C# minor
    (4, False, "12B"),  # E major
    (5, True, "4A"),    # F minor
    (7, True, "6A"),    # G minor
])
def test_alphanumeric(tonic, minor, expected):
    assert keyenergy.alphanumeric(tonic, minor) == expected


def test_alphanumeric_neighbours_are_a_fifth_apart():
    for t in range(12):
        n = int(keyenergy.alphanumeric(t, True)[:-1])
        m = int(keyenergy.alphanumeric((t + 7) % 12, True)[:-1])
        assert m == n % 12 + 1


# --- the comment -------------------------------------------------------------

@pytest.mark.parametrize("old, new", [
    (None, "8A - Energy 6"),
    ("", "8A - Energy 6"),
    ("5A - Energy 7 /* Atmo / Rytm */", "8A - Energy 6 /* Atmo / Rytm */"),
    ("  12B - Energy 10  /* x */", "8A - Energy 6 /* x */"),
    ("10B - Energy 3", "8A - Energy 6"),
    ("great tune", "8A - Energy 6 great tune"),
    ("/* Duplicate */", "8A - Energy 6 /* Duplicate */"),
])
def test_comment_keeps_everything_but_the_prefix(old, new):
    assert rekordbox.comment_with(old, "8A", 6) == new


# --- audio -------------------------------------------------------------------

def tone(freqs, seconds=10.0, amp=0.1, partials=6):
    """A sustained chord with harmonics, like a synth pad."""
    t = np.arange(int(seconds * SR)) / SR
    x = np.zeros_like(t)
    for f in freqs:
        for h in range(1, partials + 1):
            x += amp / h * np.sin(2 * np.pi * f * h * t)
    return x.astype(np.float32)


def hz(midi):
    return 440.0 * 2 ** ((midi - 69) / 12)


def chord(root, minor):
    return [hz(root), hz(root + (3 if minor else 4)), hz(root + 7)]


def test_a_minor_progression_is_8a():
    # i - iv - v - i in A minor
    windows = [tone(chord(57, True)), tone(chord(62, True)),
               tone(chord(64, True)), tone(chord(57, True))]
    result, _ = keyenergy.analyse_audio(windows)
    assert result.key == "8A"


def test_f_minor_progression_is_4a():
    windows = [tone(chord(53, True)), tone(chord(58, True)),
               tone(chord(60, True)), tone(chord(53, True))]
    result, _ = keyenergy.analyse_audio(windows)
    assert result.key == "4A"


def test_detuned_audio_gets_the_same_key():
    # a quarter tone flat is the worst case for tuning estimation
    detune = 2 ** (-0.4 / 12)
    windows = [tone([f * detune for f in chord(57, True)]),
               tone([f * detune for f in chord(62, True)]),
               tone([f * detune for f in chord(64, True)]),
               tone([f * detune for f in chord(57, True)])]
    result, _ = keyenergy.analyse_audio(windows)
    assert result.key == "8A"


def test_loud_bright_noise_is_more_energetic_than_a_quiet_pad():
    rng = np.random.default_rng(0)
    noise = [(0.3 * rng.standard_normal(10 * SR)).astype(np.float32)
             for _ in range(4)]
    pad = [tone(chord(57, True), amp=0.003) for _ in range(4)]
    loud, _ = keyenergy.analyse_audio(noise)
    soft, _ = keyenergy.analyse_audio(pad)
    assert loud.energy > soft.energy
    assert 1 <= soft.energy <= 10 and 1 <= loud.energy <= 10


def test_same_audio_same_answer():
    windows = [tone(chord(60, False)) for _ in range(3)]
    assert keyenergy.analyse_audio(windows) == keyenergy.analyse_audio(windows)


def test_version_changes_with_the_method(monkeypatch):
    before = keyenergy.version()
    monkeypatch.setattr(keyenergy, "HARMONICS", keyenergy.HARMONICS + 1)
    assert keyenergy.version() != before


# --- rekordbox ---------------------------------------------------------------

XML = """<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <COLLECTION Entries="2">
    <TRACK TrackID="1" Location="file://localhost{root}/a.mp3" Tonality="5A"
           Comments="5A - Energy 7 /* Atmo */" Colour="" Rating="0"/>
    <TRACK TrackID="2" Location="file://localhost{root}/b.mp3" Tonality=""
           Comments="" Rating="0"/>
  </COLLECTION>
  <PLAYLISTS><NODE Type="0" Name="ROOT" Count="0"/></PLAYLISTS>
</DJ_PLAYLISTS>
"""


def tracks(path: Path) -> dict:
    return {t.get("TrackID"): t for t in ET.parse(path).getroot().iter("TRACK")
            if t.get("Location")}


def test_write_sets_key_energy_and_keeps_tags(tmp_path):
    xml = tmp_path / "rekordbox.xml"
    xml.write_text(XML.format(root=tmp_path))
    st = rekordbox.write(xml, xml, tmp_path, [
        rekordbox.Entry("a.mp3", "colour", Label("Pink", 4), "8A", 6),
        rekordbox.Entry("b.mp3", key="3B", energy=9),
    ], name="t1")
    t = tracks(xml)
    assert t["1"].get("Tonality") == "8A"
    assert t["1"].get("Comments") == "8A - Energy 6 /* Atmo */"
    assert t["1"].get("Rating") == "204"
    assert t["2"].get("Comments") == "3B - Energy 9"
    assert st["keyed"] == 2 and st["commented"] == 2
    assert (tmp_path / "rekordbox.xml.bak").exists()


def test_settings_switch_each_field_off(tmp_path):
    xml = tmp_path / "rekordbox.xml"
    xml.write_text(XML.format(root=tmp_path))
    rekordbox.write(xml, xml, tmp_path, [rekordbox.Entry("a.mp3", key="8A", energy=6)],
                    tonality=False, comment=True)
    assert tracks(xml)["1"].get("Tonality") == "5A"
    rekordbox.write(xml, xml, tmp_path, [rekordbox.Entry("b.mp3", key="8A", energy=6)],
                    tonality=True, comment=False)
    assert tracks(xml)["2"].get("Comments") == ""
    assert tracks(xml)["2"].get("Tonality") == "8A"


def test_changed_only_lists_only_changes_and_leaves_a_settled_file_alone(tmp_path):
    xml = tmp_path / "rekordbox.xml"
    xml.write_text(XML.format(root=tmp_path))
    entries = [rekordbox.Entry("a.mp3", key="8A", energy=6),
               rekordbox.Entry("b.mp3", key="3B", energy=9)]
    st = rekordbox.write(xml, xml, tmp_path, entries, name="p1", changed_only=True)
    assert sum(st["playlists"].values()) == 2
    before = xml.read_bytes()
    st = rekordbox.write(xml, xml, tmp_path, entries, name="p2", changed_only=True)
    assert sum(st["playlists"].values()) == 0 and st["unchanged"] == 2
    assert xml.read_bytes() == before          # nothing to import: untouched
