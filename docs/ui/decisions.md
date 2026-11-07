# UI decisions: Studio workspace

The design behind [workspace.html](workspace.html) ([png](workspace.png)),
mirrored from the design canvas
<https://claude.ai/artifact/VGEERe7xK3zbRnQ2Etn7FS>. Each point records what
the user decided and why, so nothing gets lost while building the Godot UI.
[user-flows.md](../user-flows.md) holds the trust and calibration model this
builds on.

Update this file, `workspace.html` and `workspace.png` with every canvas
change. To regenerate the html: take `project/Main.dc.html`, keep the
`<helmet>` contents as `<head>`, and drop `support.js`, the `x-dc` wrapper and
its script.

![Studio workspace](workspace.png)

## Direction

- Of three directions (A Rack, B Studio, C Paper), **B Studio** was chosen:
  dense, dark, keyboard-first, with a lime accent (`#c8f13c`).
- Remove anything a user would not understand or act on. Removed so far: the
  "Review · batch" header, the "first run" tag, "End calibration early", the
  export warning, and the table hints and sort shortcuts on the export strip.

## Screens and rail

- The rail holds **review, library, stats, settings**, with settings directly
  below stats.
- **Settings is the setup.** On first run the user fills in Music and
  `rekordbox.xml`; review, library and stats stay greyed out until both are
  set. It is the same page afterwards. Keep "Start reviewing ENTER" for now.
- Stats and collection are one click away from every screen.

## Review workspace: one screen

The screen reads top to bottom as the flow:
queue → model → player → reviewed → table. Darker strips (`#222327`) separate
the stages, and ↓ arrows sit at both ends of each strip.

1. **Queue** (light). Cards show a 48 px cover, the tag, the title in bold and
   the artist in grey. Tags: next, conflict, calibration, unsure, re-ask, old
   label. The queue is ordered by what teaches the model most.
2. **rekordbox widget**, at the right end of the queue row with a ← pointing
   into the queue, because it is where the queue's tracks come from. It shows:
   - "watching rekordbox.xml";
   - "collection exported X ago";
   - "+N queued";
   - a "Sync ⌥S" button.
   When the export is older than the threshold (default 7 days), the widget
   turns amber and says "old: export it again in rekordbox before Export".
   There is only one timestamp, because the app syncs as soon as rekordbox
   exports.
3. **Model strip** (dark), between the queue and the player, because the tracks
   pass through the model. It shows:
   - "early calibration" plus "learning your taste, guess hidden (blind)";
   - progress, e.g. "47 / 100 labels";
   - "model guesses right 40–70%", always shown as a range, so the user sees
     everything from the start.
4. **Player** (light). Call it "player", never "card". It holds:
   - the full-track waveform with section labels, click and jump;
   - the colour keys in **two rows** (QWER light / ASDF dark), which the user
     likes;
   - stars (1–5, hold ⇧);
   - shortcuts, and the two action buttons.
5. **Reviewed strip** (dark). It shows "↓ Reviewed 47" and an **"Export → X"**
   button. The arrow is on the button and points right, because the tracks go
   out of the app. Nothing else is on the strip.
6. **Reviewed table** (light). One flat table like rekordbox, with no colour
   grouping. Click a header to sort by colour, energy, rating or BPM; click a
   row to relisten in the player and fix it. This is how the user hunts for
   outliers: sort, step through, fix.

### Player buttons: same slot, same keys everywhere

| context | ⇧ENTER | ENTER |
|---|---|---|
| review, track from the queue | **Skip track**: next track, no label | **Next**: save, next track |
| review, relistening a reviewed row | **Requeue**: back to the queue (the user prefers this short label) | **Next**: save, back to the queue |
| collection | **Skip**: next row, no save | **Next**: save, add to the reviewed table and the export, next row |

- ESC stops playback, as in rekordbox. There is no separate "back" or "close".
- The label changes with the context, but the position and the key never do.

## Calibration

- There is no button to end or skip calibration. It is just the queue's early
  phase, and the model strip shows where you are.
- In settings, a **threshold slider (100–300, default 100)** sets when batch
  tracks and corrections start mixing into the queue. Below it the queue holds
  only calibration tracks and conflicts, all blind.
- "Skip track" means only "skip this one track". Users read it that way, so it
  must never mean "skip calibration".

## rekordbox: export and sync

- **One file both ways**, as in the CLI: `~/Documents/rekordbox.xml`.
  rekordbox exports the collection over it, and vibecheck writes its playlists
  into it (keeping the old one as `.bak`). The user then just refreshes in
  rekordbox.
- **Export** can happen at any time and opens the **Export done** popup, which
  dims the workspace and says:
  - "47 tracks · vibecheck-2026-11-07", written into the xml;
  - **Now in rekordbox:**
    1. refresh the rekordbox xml node;
    2. right-click the playlist › Import Playlist;
    3. play, fix, then File › Export Collection in xml format over the same
       file, and vibecheck syncs on its own;
  - **First time only:** Preferences › Advanced › Database › rekordbox xml,
    then choose the file;
  - OK, with ENTER.
- **Sync** is automatic. The app watches the xml, and when rekordbox exports,
  the **Sync summary** popup dims the workspace and shows: played / corrected /
  untouched, plus how many tracks went to the front of the queue. "Back to
  review ENTER" confirms. "Sync ⌥S" in the widget is the manual fallback.
- Importing applies every field in the xml, so the export has to be recent.
  That is why the widget warns when it is old.
- **Canvas layout:** export goes out at the bottom (the export strip → the
  Export done popup, bottom right), and sync comes in at the top (rekordbox →
  the Sync summary, top right → confirm → the queue).

## Collection

- The player is docked **on top** and is the **same component as in review**,
  so the user recognises it and it can be reused.
- Below it: filter chips, then a selection bar ("N selected · Add to review
  queue ⌥Q"), then the **same flat table** with extra columns (plays, last
  played, added).
- Stepping through the collection with Next saves each track into the **same
  reviewed table** as review, so it goes out with the next Export. Rows that
  came from here are flagged "from library".

## Settings contents

- Library: the Music path and the `rekordbox.xml` path, with the note that the
  one file is used both ways.
- Track, labelled and conflict counts, and the progress of listening to a first
  sample.
- The calibration threshold, 100–300.
- Show the model's guess in the player after calibration (On).
- Re-ask old tracks blind (rises as the library fills in).
- Warn when the collection export is older than 7 days.

## Open

- Should the review player and the table rows show cover art too?
- Should the Export button warn or block while the collection export is old?
- Should settings later drop "Start reviewing" and move the counts to stats?
- Should the collection player fold to a slim bar when no track is open?
- Does a skipped track go back into the queue or get dropped?
