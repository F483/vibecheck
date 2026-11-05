"""Cue points: the grid, placement and its inverse, whose cues may be touched,
and writing them into the rekordbox xml."""

from __future__ import annotations

import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np
import pytest

from vibecheck import cues, rekordbox

PAIR, LOOPS = [16, 8], {"A": 16, "G": 16}


# --- the grid ----------------------------------------------------------------

def test_downbeats_follow_the_grid():
    # 120 bpm from 0.5 s, first beat a downbeat: a bar every 2 s
    d = cues.downbeats([(0.5, 120.0, 1)], 10.0)
    assert np.allclose(d, [0.5, 2.5, 4.5, 6.5, 8.5])


def test_downbeats_respect_where_the_bar_starts():
    # the first grid beat is beat 3 of its bar: the bar starts two beats later
    d = cues.downbeats([(0.0, 120.0, 3)], 6.0)
    assert np.allclose(d, [1.0, 3.0, 5.0])


def test_the_grid_extends_back_to_the_start():
    d = cues.downbeats([(4.1, 120.0, 1)], 8.0)
    assert np.allclose(d, [0.1, 2.1, 4.1, 6.1])


def test_tempo_changes_start_a_new_section():
    d = cues.downbeats([(0.0, 120.0, 1), (4.0, 60.0, 1)], 12.0)
    assert np.allclose(d, [0.0, 2.0, 4.0, 8.0])


# --- placement and its inverse -----------------------------------------------

def grid(n=200):
    return np.arange(n) * 2.0


def hot(marks):
    return {cues.SLOTS[int(m["Num"])]: m for m in marks if int(m["Num"]) >= 0}


def test_pairs_fill_the_slots_by_role():
    h = hot(cues.place([32, 64, 80, 112], grid(), PAIR, LOOPS, False))
    bars = {k: round(float(m["Start"]) / 2) for k, m in h.items()}
    assert bars == {"A": 16, "B": 24, "C": 48, "D": 56,
                    "E": 64, "F": 72, "G": 96, "H": 104}
    assert h["A"]["Type"] == "4" and float(h["A"]["End"]) == 32 * 2.0
    assert h["G"]["Type"] == "4" and h["B"]["Type"] == "0"
    assert (h["A"]["Red"], h["B"]["Green"]) == ("255", "226")


def test_three_transitions_keep_the_mix_out_on_g_and_h():
    h = hot(cues.place([32, 64, 112], grid(), PAIR, {}, False))
    assert sorted(h) == ["A", "B", "C", "D", "G", "H"]


def test_the_default_cues_at_each_change():
    h = hot(cues.place([32, 64, 80, 112], grid(), [0], {}, False))
    assert {k: float(m["Start"]) for k, m in h.items()} == {
        "A": 64.0, "B": 128.0, "C": 160.0, "D": 224.0}


def test_memory_cues_copy_the_hot_cues_and_mark_the_start():
    marks = cues.place([32, 64, 80, 112], grid(), PAIR, LOOPS, True)
    mem = [m for m in marks if m["Num"] == "-1"]
    assert len(mem) == 9 and mem[0]["Start"] == "0.000"
    assert not any("Red" in m for m in mem)
    assert sum(m["Type"] == "4" for m in mem) == 2


def test_a_cue_before_the_track_starts_lands_on_the_first_bar():
    # 16 and 8 bars before bar 8 are both bar 0: one cue, not two
    h = hot(cues.place([8, 64, 80, 112], grid(), PAIR, {}, False))
    assert float(h["A"]["Start"]) == 0.0 and "B" not in h


@pytest.mark.parametrize("lead", [[16, 8], [0], [8]])
def test_placement_and_reading_back_agree(lead):
    T = [32, 64, 80, 112]
    marks = cues.place(T, grid(), lead, {}, True)
    bars = [cues.bar_of(grid(), float(m["Start"])) for m in marks if int(m["Num"]) >= 0]
    assert cues.transitions_from(bars, lead) == T


def test_a_cue_outside_any_pair_says_nothing():
    # pairs at 16/24 and 40/48; 56 and 90 are on their own
    assert cues.transitions_from([16, 24, 40, 48, 56, 90], PAIR) == [32, 56]


def test_decode_takes_four_bars_apart_with_the_mix_in_first():
    rng = np.random.default_rng(0)
    s = rng.standard_normal((120, 3))
    s[20, 1] = s[100, 2] = 50
    T = cues.decode(s)
    assert len(T) == 4 and T[0] == 20 and T[-1] == 100
    assert min(np.diff(T)) >= cues.GAP


# --- whose cues --------------------------------------------------------------

def mark(name="", num=0, start="10.000"):
    return {"Name": name, "Type": "0", "Start": start, "Num": str(num)}


def test_generated_cues_are_writable_and_yours_are_not():
    w = lambda *m: cues.writable(cues.Marks([(0.5, 120.0, 1)], 300.0, list(m)))
    assert w()
    assert w(mark("Cue 1"), mark("Cue 2", -1))
    assert not w(mark("Cue 1"), mark(""))
    assert not w(mark(""))
    # what clearing leaves: one memory cue on the first downbeat
    assert w(mark("", -1, "0.500"))
    assert not w(mark("", -1, "10.500"))


def track_with(lead_bars):
    tempos = [(0.0, 120.0, 1)]
    return cues.Marks(tempos, 400.0, [mark(num=i, start=f"{b * 2.0:.3f}")
                                      for i, b in enumerate(lead_bars)])


def test_only_your_own_cues_are_learned_from():
    own = track_with([16, 24, 48, 56, 96, 104])
    untouched = track_with([16, 24, 48, 56, 96, 104])
    moved = track_with([16, 24, 48, 56, 88, 96])
    gen = cues.Marks([(0.0, 120.0, 1)], 400.0,
                     [mark(f"Cue {i + 1}", i, f"{b * 2.0:.3f}")
                      for i, b in enumerate([16, 24, 48, 56, 96, 104])])
    written = {"untouched": cues.hot_starts(untouched.marks),
               "moved": cues.hot_starts(own.marks)}
    got = cues.yours({"own": own, "untouched": untouched, "moved": moved,
                      "generated": gen}, written, PAIR)
    assert got == {"own": [32, 64, 112], "moved": [32, 64, 104]}


# --- the xml -----------------------------------------------------------------

XML = """<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <COLLECTION Entries="1">
    <TRACK TrackID="1" Location="file://localhost{root}/a.mp3" TotalTime="300">
      <TEMPO Inizio="0.050" Bpm="128.00" Metro="4/4" Battito="1"/>
      <POSITION_MARK Name="Cue 1" Type="0" Start="0.050" Num="0"/>
      <POSITION_MARK Name="Cue 1" Type="0" Start="0.050" Num="-1"/>
    </TRACK>
  </COLLECTION>
  <PLAYLISTS><NODE Type="0" Name="ROOT" Count="0"/></PLAYLISTS>
</DJ_PLAYLISTS>
"""


def test_reading_and_replacing_cues(tmp_path):
    xml = tmp_path / "rekordbox.xml"
    xml.write_text(XML.format(root=tmp_path))
    m = rekordbox.read_marks(xml, tmp_path)["a.mp3"]
    assert m.tempos == [(0.05, 128.0, 1)] and m.duration == 300.0
    assert cues.generated(m.marks)

    new = cues.place([32, 64, 80, 112], cues.downbeats(m.tempos, m.duration),
                     PAIR, LOOPS, True)
    st = rekordbox.write(xml, xml, tmp_path, [rekordbox.Entry("a.mp3", cues=new)],
                         name="p", changed_only=True)
    assert st["cued"] == 1
    t = next(t for t in ET.parse(xml).getroot().iter("TRACK") if t.get("TrackID"))
    assert [dict(p.attrib) for p in t.findall("POSITION_MARK")] == new
    assert t.find("TEMPO") is not None

    # the same cues again change nothing, and leave the file alone
    before = xml.read_bytes()
    st = rekordbox.write(xml, xml, tmp_path, [rekordbox.Entry("a.mp3", cues=new)],
                         name="p2", changed_only=True)
    assert st["cued"] == 0 and xml.read_bytes() == before


def test_the_feature_version_changes_with_the_method(monkeypatch):
    before = cues.version()
    monkeypatch.setattr(cues, "KS", (3, 4))
    assert cues.version() != before
