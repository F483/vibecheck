"""Reading and writing the label in an mp3's ID3 genre frame.

The only field this app touches. Writing is in place -- the ID3 tag is
rewritten, the audio frames are not -- which is why track identity hashes the
audio region only (store.partial_hash): a label written here must not change
what the track *is*.
"""

from __future__ import annotations

from pathlib import Path

from mutagen.id3 import ID3, ID3NoHeaderError, TCON


def read(path: Path) -> str | None:
    try:
        tag = ID3(path)
    except ID3NoHeaderError:
        return None
    except Exception:
        return None
    frame = tag.get("TCON")
    if frame is None or not frame.text:
        return None
    value = str(frame.text[0]).strip()
    return value or None


def write(path: Path, label: str | None) -> None:
    try:
        tag = ID3(path)
    except ID3NoHeaderError:
        tag = ID3()
    if label is None:
        tag.delall("TCON")
    else:
        tag.setall("TCON", [TCON(encoding=3, text=[label])])
    tag.save(path)
