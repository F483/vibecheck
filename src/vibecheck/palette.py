"""The palette, which is fixed.

Rekordbox exposes exactly eight colour tags and six star ratings, and that is
not a matter of taste -- it is the target software's UI. What is personal is
*which track gets which colour*, and that is learned, never declared.

An earlier design kept labels as opaque strings so any vocabulary would work.
That was generality about the wrong thing: it bought vocabulary-independence
nothing will ever use, and paid for it by making hue, tone and rating
unqueryable. The structure lives here, typed; the taste lives in the model.

Hue and tone are a property of the eight, not an opinion: they form a 4x2 grid
whose cells are the colours, so a confident hue plus a confident tone *is* a
colour.
"""

from __future__ import annotations

from typing import NamedTuple


class Colour(NamedTuple):
    id: int
    name: str
    hue: str
    tone: str
    rgb: str          # as rekordbox writes it in its XML


COLOURS: tuple[Colour, ...] = (
    Colour(1, "Red",    "Warm",    "Dark",  "0xFF0000"),
    Colour(2, "Orange", "Warm",    "Light", "0xFFA500"),
    Colour(3, "Yellow", "Acidic",  "Light", "0xFFFF00"),
    Colour(4, "Green",  "Acidic",  "Dark",  "0x00FF00"),
    Colour(5, "Aqua",   "Cool",    "Light", "0x25FDE9"),
    Colour(6, "Blue",   "Cool",    "Dark",  "0x0000FF"),
    Colour(7, "Purple", "Vibrant", "Dark",  "0x660099"),
    Colour(8, "Pink",   "Vibrant", "Light", "0xFF007F"),
)

HUES = ("Warm", "Acidic", "Cool", "Vibrant")
TONES = ("Dark", "Light")
MAX_STARS = 5
STAR_VALUES = tuple(range(MAX_STARS + 1))    # 0 = unrated, as rekordbox means it

BY_ID = {c.id: c for c in COLOURS}
BY_NAME = {c.name: c for c in COLOURS}
BY_RGB = {c.rgb: c for c in COLOURS}
BY_GRID = {(c.hue, c.tone): c for c in COLOURS}

# rekordbox stores a rating as 0, 51, 102, 153, 204 or 255
STAR_STEP = 51


def stars_from_rating(raw: str | int | None) -> int:
    return int(raw or 0) // STAR_STEP


def rating_from_stars(stars: int) -> str:
    return str(stars * STAR_STEP)


def colour_from_grid(hue: str | None, tone: str | None) -> Colour | None:
    """Both axes, or nothing: one alone narrows the choice without making it."""
    if hue is None or tone is None:
        return None
    return BY_GRID.get((hue, tone))


def _check() -> None:
    assert len(COLOURS) == 8 and len(BY_GRID) == 8, "the grid must be exact"
    assert set(c.hue for c in COLOURS) == set(HUES)
    assert set(c.tone for c in COLOURS) == set(TONES)


_check()
