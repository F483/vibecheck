"""m3u8 read and write.

Plain absolute paths on write. Rekordbox strips a `file://` scheme but does not
percent-decode, so URI-encoded entries fail to import every single track -- it
looks for a literal path containing `%20`.

That leaves one unavoidable gap: a `#` anywhere in a path appears to start a
comment for Rekordbox's parser, so those tracks silently fail to import. 41 of
14,194 paths in the reference collection are affected. The writer reports them
rather than letting them disappear unnoticed.

Reading is forgiving in both directions -- playlists come back from software
this app does not control.
"""

from __future__ import annotations

import urllib.parse
from pathlib import Path


def write(path: Path, root: Path, entries: list[tuple[str, str]]) -> list[str]:
    """entries: (relative path, display title). Returns paths likely to fail."""
    lines = ["#EXTM3U"]
    risky = []
    for rel, title in entries:
        lines.append(f"#EXTINF:-1,{title}")
        lines.append(str(root / rel))
        if "#" in rel:
            risky.append(rel)
    path.write_text("\n".join(lines) + "\n")
    return risky


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
