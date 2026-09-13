"""m3u8 read and write.

Paths are percent-encoded `file://` URIs on write: 41 tracks in the reference
collection contain a `#`, which truncates a bare path when anything reads it as
a URI, and those tracks then vanish from the playlist with no error at all.

Reading is forgiving in the other direction -- playlists come back from
software this app does not control.
"""

from __future__ import annotations

import urllib.parse
from pathlib import Path


def write(path: Path, root: Path, entries: list[tuple[str, str]]) -> None:
    """entries: (relative path, display title)"""
    lines = ["#EXTM3U"]
    for rel, title in entries:
        lines.append(f"#EXTINF:-1,{title}")
        lines.append("file://" + urllib.parse.quote(str(root / rel)))
    path.write_text("\n".join(lines) + "\n")


def read(path: Path, root: Path) -> list[str]:
    """Return relative paths, skipping anything outside the collection."""
    out = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("file://"):
            line = urllib.parse.unquote(line[7:])
            if line.startswith("localhost"):
                line = line[len("localhost"):]
        try:
            out.append(str(Path(line).resolve().relative_to(root.resolve())))
        except ValueError:
            continue
    return out
