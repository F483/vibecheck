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

**Built in Godot** in `app/`, on the core. `app/main.gd` holds the rail, the
dialogs and key routing. Each screen is in `app/ui/`, along with the shared
`player.gd` and `table.gd` (drawn, so 15,000 rows scroll freely), and `ui.gd`
holds the palette and widgets. All of it is built in code: no `.tscn` per
screen and no imported assets. `state.gd` calls the core through the
extension's one JSON call (`ext/src/lib.rs` → `core/src/app.rs`) on worker
threads. Screenshot any screen with
`godot --path app -- --shot=out.png --screen=library --dialog=export`; add
`--music=… --xml=…` (and `VIBECHECK_STATE=<dir>`) to try a copy without
touching the saved settings or the collection's state.

How the app maps onto the core:

- A decision in the player is a `user` label outside any round. What was
  labelled since the last export is the export list and the next export.
  The export is remembered in `.vibecheck/app.json`, with each track's play
  count at the time, so the next sync can count plays.
- Sync is the command line's, plus two rules: a guess the app exported that
  comes back unchanged stays a guess (only the command line's open rounds
  are confirmed that way); and a track labelled after the xml was
  exported is left alone, since the xml can't know about it.
- Start fast, close fast. The app starts on the last trained model (about 1 s
  to the first track) and, if your labels moved on since, retrains behind it
  and refreshes the queue; the library and the first queue load side by side.
  Closing stops long work at its next safe point, waits only for what writes
  labels, and ends the process (tens of ms): background listening only writes
  the cache, where an interrupted write rolls back. Ctrl+C in the terminal
  takes the same way out.
- The window can't be made smaller than the largest screen needs (now about
  1164×691 points, set by the collection table's columns), so nothing is
  ever pushed out of view. It is measured from the screens; the queue cards
  (as many as fit), the player's title (trimmed with …) and the artist line
  don't count towards it. It opens at 1280×720 (trying it out; 1360×800 was
  the other candidate).
- The model in use is trained on the training slice only, so the accuracy it
  shows is measured on itself (held-back labels, 95% range).
- Queue: calibration tracks spread across the sound of the collection while
  below the threshold; after it, the least sure, mixed with conflicts and old
  labels, asked blind.
- Player sections are the cue model's transitions (mix in, changes, mix out)
  when rekordbox has a beat grid.
- Trust is approximated until schema v5: played = a label and a play count.

## Direction

- Of three directions (A Rack, B Studio, C Paper), **B Studio** was chosen:
  dense, dark, keyboard-first, with a lime accent (`#c8f13c`).
- Remove anything a user would not understand or act on. Removed so far: the
  "Review · batch" header, the "first run" tag, "End calibration early", the
  export warning, and the table hints and sort shortcuts on the export strip.

## Screens and rail

- The rail holds **vibe, library, stats, history, settings**, with settings
  last. Each item is an **icon** with a small label underneath:
  sparkles for vibe (it stands for the AI; chosen over headphones,
  a brain, a node graph and a waveform), books for library, a chart for stats,
  a clock with an arrow running back for history, a gear for settings. These replace the earlier letters (R/C/S/,). The icons are
  18 px line drawings: grey normally, lime when active.
- No "v" logo on top of the rail: it did nothing (the user). The window
  title names the app.
- **Status row** on top of every screen, right of the rail (the user): the
  model strip at the left, the rekordbox widget at the right, one line
  (32 px). It gathered what sat on vibe, then for a while on Stats as two
  strips under a "Stats" title, which went (the user). Without a
  rekordbox.xml it reads "rekordbox not set".
- **Settings is the setup.** On first run the user fills in Music and
  `rekordbox.xml`; vibe, library and stats stay greyed out until both are
  set. It is the same page afterwards. Keep "Start ENTER" for now.
- Stats, history and collection are one click away from every screen.
- Stats opens at once: what it shows comes from the cached model. The "colour right vs
  labels trained on" curve trains a model per point (seconds), so it is measured
  after each retrain in the background and kept by your labels; when your labels
  are ahead of it, the last curve shows dimmed with "measuring…" until the new
  one is in (the user found the page slow to open).
- The curve has **four lines**: colour, hue, tone and stars right, each named
  at its end with where it is now (the user). Colour is hue and tone together
  (a cell of the 4×2 grid), so it sits lowest, and hue and tone show which half
  of a wrong colour the model misses; stars count exact. All four come from the
  same models, so they cost nothing more to measure. A checkbox per line below
  the chart, its colour beside it, is the legend and hides or shows it; all
  four start ticked; the choice lasts the session, not saved (the user). Line colours are not the
  label colours' hues where it matters: lime, soft blue, soft orange, white.
- The chart is always **0–100%**: a floor that followed the lowest line made
  everything jump when a line was ticked off (the user). Labels trained on
  run on a **log scale** (each point doubles them), with dotted lines at 100,
  500, 1,000, 5,000… and the right edge a little past now. The "min 100" and
  "goal 300" marks went with the old linear axis.
- A thinner line per colour, in the label colours, with its own row of
  checkboxes (no note on how noisy they are: the user). They start **unticked** (the user: there to look at, too noisy to
  show by default). Chart 320 px tall: 480 px was more than it needed (the
  user).
- **Your consistency** on the chart (the user; first called "past self",
  renamed as clearer): a dashed grey line, ticked by default,
  for how often a track you labelled again kept its colour. Like the model
  lines it is a running total (the user): each point counts every relabel made
  while you had up to that many labels outside the holdout, from 20 relabels
  on. It is about as high as the colour line can be expected to get: the model
  can't agree with you more than you agree with yourself. Open: every Confirm
  counts as a relabel, also with the colour in view, which inflates it.
- No tiles above the chart (colour right, stars right, past self, labels):
  the chart has them all (the user). No title row: the status row is on top
  (the user). No label count or guesses-right range there either: the chart
  has both (the user).
- The chart comes first; the misses sit beside Trust and Colours (the
  user). "Mixed up most" with "Blue taken for Green 6%" left unclear who
  took what, and 6% was of all held-back labels. Now titled "Misses · model
  guessed → you confirmed", the words the player uses (the user), one per row
  so the rank reads top down: "■ Green → ■ Blue" on the left, "5 / 18 · 28%"
  on the right (guessed as the first, of your held-back labels of the second,
  and the share). The four worst by that
  share lead (the user); on few labels a share can be chance. The score
  keeps every mix-up; the model cache key went to m3 so old scores, with
  three, are refit once.
- Below the chart the page splits in two halves (the user): Trust and
  Colours on the left, the misses on the right; legends wrap.
- **Colours** under Trust (the user): the same split bar, one
  part per colour in the label colours, from all tracks with a colour (guesses
  included), with counts below and "no colour N".

## Vibe workspace: one screen

The main screen is called **Vibe**, after the app's name. It was chosen over
Review, Tag, Train, Check, Label and Listen. The list under the player is
the **export list**: everything that goes out with the next Export (and
becomes a rekordbox playlist), confirmed, guessed or dropped in as it is. It
was "Reviewed" until dropped tracks and guesses made that wrong; chosen over
Batch, Next export, Playlist and Crate.

The screen reads top to bottom: player → main controls → queue → export
list. It was queue → model → player, as the flow; the queue moved under the
controls so the player sits at the top, in the same place, on vibe and in the
library, and shows on vibe only; the model strip moved to the top of Stats
(the user). Lighter strips (`#2a2b2f`) separate the stages, which sit on the dark background (`#222327`), and ↓ arrows sit at
both ends of each strip. This was the other way round until the user asked to
invert it app-wide: the trust rings read much better on the dark background
(as in their popup), and so does everything else. The same holds on every
screen: content dark; the rail, strips, toolbars and cards lighter; the
waveform and charts darker still (`#1c1d20`); dialogs and popups unchanged.

1. **Queue** (dark). Cards show a 48 px cover, the tag, the title in bold and
   the artist in grey. The first card is outlined only while its track is in the
   player, not while you relisten to the export list (the user). The queue holds the next 200, rebuilt as the model learns; its
   title line says how many are queued and how many could be asked in all
   ("Queue 200 of 8,412": tracks listened to and not labelled by you). That was a
   "+197 of 8,412" box after the last card; moved up so the row fits one more card
   (the user). Tags: next, conflict, calibration, unsure, re-ask, old
   label. The queue is ordered by what teaches the model most. A **Fast
   track** toggle (⌥F) before the title switches it to the model's surest
   guesses, most sure first, with the library's colour / energy / rating / BPM
   chips over the cards applied to the guess; every 10th card is asked blind
   and every 20th re-asks a guess you kept, so accuracy and anchoring stay
   measured (`docs/fast-track.md`); the training queue too asks a sure track
   blind every 10th card, so the gate can be earned. Locked until sure guesses are measurably
   right, the toggle sits dimmed and the title line says what it waits on.
   Fast-track cards are tagged "SURE 94%", check cards "BLIND". **Click the
   player's tag** for a popup like the trust one: every tag and why it puts a
   track in front of you, this one lit, and what to do with it ("Pick your
   colour again to keep it, or another to correct it") (the user).
2. **rekordbox widget**, now the right half of the **status row** on every
   screen (the user). It was one line at the right of the vibe queue's
   title line, because it is where the queue's tracks come from, and before
   that a two-line box at the end of the card row with a ← into the queue.
   It shows "rekordbox · watching rekordbox.xml · collection exported X ago"
   and a "Sync ⌥S" button. "+N queued" (conflicts and old labels the last
   sync put in the queue) was dropped: History shows what each sync brought
   (the user). When the export is older than the threshold
   (default 7 days), the date turns amber and "⚠ old: export it again in
   rekordbox before Export" follows it.
   There is only one timestamp, because the app syncs as soon as rekordbox
   exports.
3. **Model strip**, now the left half of the **status row** on every screen
   (the user); it was between the queue and the player, then under the
   queue, then on Stats. No ↓ arrows any more. It shows only:
   - at the left, what it does in the background, as one line the console
     prints too (at most once a minute): "analysing ▬▬▬ 62% · 1,940 / 3,120 ·
     ~2 h left", "analysis off …" or "all 3,120 analysed" (was listening /
     heard; "analyse" is rekordbox's word for it, the user chose it). Only
     analysed tracks can be asked about, so how far along it is and how long it will take
     is what is worth knowing (the user). The "early calibration" / "mixing
     in" tags and their explanations were dropped: not useful.
   At its right were the labels ("calibrating 47 / 100 labels", "1,119
   labels") and "model guesses right 40–70%"; dropped on Stats, as the chart
   below shows both (the user).
4. **Player** (dark). Call it "player", never "card". It holds:
   - the track's **cover in front of the waveform**, as tall as the peaks (a
     gradient in the track's colours when the file has none);
   - the full-track waveform with section labels (parts too close together
     keep only their number, or no label, rather than overlap), click and jump, and the
     **cue points drawn as rekordbox draws them**, so it is clear where they
     will end up: lettered hot cues along the top (green cue, orange loop with
     its length shaded), red memory-cue triangles along the bottom. They are
     what an export would write, from the cue model and the cue settings; a
     track with cues of your own shows those, marked "your cues · kept on
     export";
   - the colour keys in **two rows** (QWER light / ASDF dark), which the user
     likes;
   - the colour keys left of the player's centre line and the tray and parts
     right of it, so on a wide window they stay centred as a pair over the
     controls tray (the user);
   - beside the colour keys, level with the bottom row, **the parts of the track**: ←
     then a button per part numbered as on the waveform, the part playing
     lit, →;
   - two trays (dark, thin border, rounded), so the groups read at a glance
     (the user asked for both; before, what you say sat level with the light
     keys and the controls on the two sides):
     - beside the colour keys, level with the top row (above the parts), **what you say about the
       track**. It had a row of its own between the waveform and the keys; moved up
       beside them for space (the user: the export list gets two more rows).
       Stars
       (five ★/☆, filled up to the rating; click or ⇧1–5), **Energy** and **Key**,
       each a value that pops out a picker (the user preferred this to − and
       + buttons): energy 1–10 in a row (N; ←→ step, a digit picks, 0 is 10),
       key all 24, minor on top and major below, round the wheel (K; ←→ round
       the wheel, ↑↓ minor/major). A key badge next to a control does the
       same as the control when clicked: the user clicked N and K expecting
       that. A corrected value turns lime. The player shows what an export
       would write: your correction, else the analysis. Thin dividers between
       stars | energy | key, as in the controls tray (the user). The **BPM** from
       rekordbox's grid is read only, so it sits in the header in front of the
       time ("126 BPM 2:41 / 6:58"); it was in this tray for a while, the user
       moved it back;
     - **the main controls**, centred: on vibe on the export list's strip,
       with Export staying at its right end (the user: it frees a row for the
       list); in the collection under the colour keys: **Undo** (U), a button
       of its own, leftmost | Stop (ESC) and Play/Pause (SPACE) | Skip or
       Requeue and Confirm. Coloured by role, so each is spotted at once
       (the user picked this from five schemes): moving through tracks slate
       `#4a5468` (Undo, Skip/Requeue), playback light grey `#d6d7db` (Stop,
       Play/Pause), deciding green `#3ddc5a` (Confirm; lime at first, the
       user asked for green), output aqua `#25e0d0` (Export).
       Rekordbox's own colours were dropped from Stop (orange) and Play
       (green): next to the colour keys they read as labels. Play/Pause and
       Skip/Requeue keep a fixed width so the tray doesn't shift.
     - With no track open the trays are hidden.
   - The artist line in the header takes the room left and trims with …, so a
     long one never pushes the player past the window.
   - In the library the hint "Confirm adds it to the next Export" sits on the
     title line, before the time, because the upper row is full.
   - **One player for vibe and library** (the user): it moves, with its
     controls strip under it, to the screen shown, so changing screen keeps the track and
     keeps it playing. Confirm
     and Skip belong to the screen in view, whatever the track: in the
     library, Confirm puts it in the export list and opens the row after the
     selected one (the first when none is), Skip opens that row; in vibe,
     Confirm or Skip on a library track goes on with the queue. In the
     library Skip never says Requeue.
   - **Main controls strip** (light), right under the player, as in the
     library. They rode centred on the export list strip until the queue
     moved below them.
5. **Export list strip** (light): the **library's search and filter row**
   (same chips, same popups, `filters.gd`), its count the export list's
   tracks shown ("47 tracks"), then an **"Export → X"** button at the right
   end (the user). It was "↓ Export list 47" and the button. The filters
   only narrow the table: Export still writes the whole list. "/" focuses
   the search here too. The arrow is on the button and points right,
   because the tracks go out of the app.
6. **Export list** (dark). Its first column is **#**, the order tracks joined
   the list (dropped, confirmed or guessed), like a rekordbox playlist's
   number: sort by it to get each drop back together. Tracks dropped in are
   marked (lime bar, faint lime row) wherever the sort puts them, until you
   open them; the user chose this over pinning them to the top. One flat
   table like rekordbox, with no colour grouping. Click a header to sort by colour, energy, rating, BPM or trust
   (the ring column's header is a ring; ⌥C ⌥E ⌥R ⌥B ⌥T); click a
   row to relisten in the player and fix it. This is how the user hunts for
   outliers: sort, step through, fix. **Click a trust ring** (here and in the
   library) for a popup that explains it: the four levels (played, set by
   confirmed, existing, guessed) and what each counts for, this track's level
   lit, and what would raise it ("Play it in a set to make it count most").
   The ring is the one new idea to learn (user-flows), so it explains itself
   where it is shown; clicking it doesn't open the track. A lime ▶ at the start of the row marks
   the track in the player; there is no status column ("just reviewed", "in
   the player", "from library" were dropped as noise). The track just saved
   keeps a faint highlight.

### Player buttons: same slot, same keys everywhere

| context | ⇧ENTER | ENTER |
|---|---|---|
| vibe, track from the queue | **Requeue**: to the end of the queue, asked again later; next track, no label | **Confirm**: your label, next track |
| vibe, relistening a row of the export list | **Skip**: next row in the list as sorted now, no label; after the last, back to the queue. (Was "Requeue" here and "Skip track" on the queue; swapped, since the queue one is what puts the track back in the queue) | **Confirm**: your label, next row in the list as sorted now; after the last, back to the queue. Sort, then work through the list top to bottom |
| collection | **Skip**: next row, no label | **Confirm**: your label, into the export list, next row |

- ESC stops playback and returns to the start of the track, as Stop; SPACE
  pauses where it is. There is no separate "back" or "close".
- The label changes with the context, but the position and the key never do.
- **Keyboard layouts.** The colour grid and the number row are matched by
  position, so they stay a grid on any layout, and their badges show what the
  keyboard in use prints there (on Neo 2: X V L C / U I A E). Every other
  shortcut is matched by the letter it types (N, K, U, X, ⌥S, …). Where a
  letter lands on a colour position, the colour wins: on Neo 2 that is U (the
  Red position) and X (Orange), so undo is also ⌘Z, the same on every layout.
  The user types on Neo 2.
- ENTER is **Confirm**. Before it was "Save" (the user found it confusing:
  it sounds like storing a file, not like the values becoming yours) and
  before that "Next" (undersold it). "Confirm" was first rejected as sounding
  like accepting the model's guess, but most values are pre-filled now
  (guesses, list rows), and on a blind track you confirm your own picks; the
  blind setting, not the word, guards against anchoring. Considered too:
  Done, Label, Keep, Approve/Accept, Set.
- Trust level names follow: **confirmed** (was "set by hand") and
  **guessed** (was "untouched", unclear). The sync summary still says
  "untouched", since there it is what happened in rekordbox ("still
  guessed").

### Drag in from rekordbox

- Select tracks in rekordbox and drag them onto the vibe screen. Where they
  land decides what happens:
  - **on the export list or its strip**: they join the list and the **next
    Export** as they are, every row alike (the user: "users will not care
    about this distinction; the trust ring says whether it was confirmed").
    A track with no colour gets the model's guess for colour and stars, saved
    as its label (guessed); one with a rekordbox colour keeps it (existing,
    or played). The export remembers those rekordbox colours: changed in
    rekordbox by the next sync, it becomes your label; left alone, it stays
    as it was. The first opens in the player ("from rekordbox"); Confirm moves
    down the list, Skip to the next dropped row. Kept across restarts in the
    collection's `.vibecheck/app.json` until exported. (Before, dropped rows
    with a colour were dimmed and held back until confirmed; dropped.) No
    guesses while calibrating or with the guess turned off; a track the model
    hasn't listened to is listened to first, a few seconds each, and while it
    waits its **trust ring spins** (a lime arc going round), turning into the
    guessed ring when the guess lands (chosen over a moving dot in the
    colour cell, a "listening…" word and a progress bar);
  - **anywhere else**: to the **front of the queue**, in the order dragged,
    tagged "from rekordbox", the first straight into the player, also as they
    are, not blind. They stay at the front when the queue is rebuilt, until
    confirmed or skipped; "Add to queue" from the library works the same way,
    after the track playing.
- rekordbox drags the files themselves (it copies them when dropped on a
  folder), so the app gets their paths and matches them to the library,
  compared normalised since macOS may hand names over decomposed. The drop
  carries no position, so the app reads where the mouse let go.
- Files outside the collection are listed in a short notice, not added.

## Calibration

- There is no button to end or skip calibration. It is just the queue's early
  phase, and the model strip shows where you are.
- In settings, a **threshold slider (100–300, default 100)** sets when batch
  tracks and corrections start mixing into the queue. Below it the queue holds
  only calibration tracks and conflicts, all blind.
- No button reads "skip calibration": Requeue and Skip only ever move past
  one track.

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
  vibe ENTER" confirms. "Sync ⌥S" in the widget is the manual fallback.
- Importing applies every field in the xml, so the export has to be recent.
  That is why the widget warns when it is old.
- Before a sync, the app no longer refuses an xml exported before its newest
  guess (the command line still does, unless `--stale-ok`): guesses for
  dropped tracks are saved at any time, so that is normal.
- **A sync leaves alone what rekordbox can't know yet:** a track labelled or
  guessed here and still waiting for the next export (the export list and the
  dropped tracks). Its value in the xml is the old one, not a correction; taking
  it blanked labels made in the app (the user hit this). Labels read from the
  xml are marked (`origin = 'xml'`, schema v7), so a change made in rekordbox
  after a sync took one is still taken.
- **Canvas layout:** export goes out at the bottom (the export strip → the
  Export done popup, bottom right), and sync comes in at the top (rekordbox →
  the Sync summary, top right → confirm → the queue).

## History

- A rail page of its own (the user's choice over a tab on stats) so it is
  clear **what came in and what went out**: syncs on the left (**In from
  rekordbox**), exports on the right (**Out to rekordbox**), newest first.
- No "History" title row (the status row is on top): each column's title is
  the title, larger (15 px semibold, was small caps), with its count right of
  it, "In from rekordbox  7 syncs", "Out to rekordbox  2 exports": no · dots,
  they looked odd (the user).
- An **In** card: when it ran and when rekordbox wrote the xml; corrected
  (ring 3), played (ring 4), added; then confirmed and "left alone, not
  exported yet" in grey. Syncs that changed nothing fold into one grey line, "3
  syncs, no changes", with their time span: the app syncs on every rekordbox
  export, so most are empty.
- An **Out** card: the playlist name and time; tracks, keys, energy
  comments, cues written; then what became of its tracks as later syncs saw
  them: played, corrected, untouched, with the sync summary's rings. A track
  played stays played; a correction outranks untouched. "not synced since"
  until a sync has looked.
- **Click a card** for its tracks in a popup over the page: colour swatch and
  stars (before → after for a correction), title — artist, and for an export
  the key, cue count and fate. Close with ENTER.
- **View only**: no undo of a batch from here (the user's choice).
- Kept in `labels.db` (schema v6, `batches` and `batch_tracks`), so it travels
  with the music like the labels. It starts when this was built; the last
  export from before is taken from `app.json`, with its tracks' labels as they
  are now.

## Collection

- The top is **vibe's own player** and its main controls strip, moved over as
  they are, in the same place as on vibe. No queue or model line here: they
  are vibe's only (the user). Changing screen moves nothing up there and the track
  keeps playing (the user: the player jumping was jarring). The search bar
  and its useless Columns button on top were dropped; search sits left of
  the filter chips (/ still focuses it).
- The search is a **field**: a darker box (`#1c1d20`) with a grey border,
  lime border and caret while typing. Bare placeholder text on the strip
  didn't read as somewhere to type (the user). Its placeholder is just
  "Search tracks": the count already sits at the right end of the row.
- **Filter chips** open a popup under them, the player's energy and key
  picker (the user: they were dead). Each is a **multi-select**, as the
  colours (the user): a click turns a value on or off and the popup stays
  open; "any" ("all" for colours) clears it and closes. Colours (and "no
  colour"), energy 1–10, rating none and 1–5 stars, BPM in tens (70 holds
  all below, 180 all above). The chip says what is picked, runs joined
  ("Energy 4–6, 9", "BPM 120–139"; one colour by name, "Yellow", more as
  "3 colours", the user), and is lime while set. Energy and BPM
  were two-click ranges and rating "at least", which closed on the second
  click. No export list filter: the vibe screen is the export list (the
  user).
- Below it: search and filter chips, then a selection bar ("N selected · Add to
  queue ⌥Q"), then the **same flat table** with extra columns (plays, last
  played, added).
- Stepping through the collection with Confirm puts each track into the **same
  export list** as vibe, so it goes out with the next Export.

## Settings contents

- **Two columns**, so lines don't stretch across a wide window: the
  collection and the model on the left (Library, Calibration), what goes into
  rekordbox on the right (Rekordbox Export, Cue points). Start spans both.
- **Each group on its own card**, titled, with one line on what it is for where
  that isn't plain (none on Library or Rekordbox Export), so
  a new user sees four groups rather than a wall of settings. Fields inside a
  card sit darker. No "Settings" title row and no intro line: the
  rail says where you are, and its dimmed screens that they wait on setup. The core version sits in small type under Start.
- Library: the Music path and the `rekordbox.xml` path, with the note that the
  one file is used both ways.
- The progress of analysing the collection. No count tiles: the status row
  and stats carry those.
- The calibration threshold, 100–300 ("Calibration ends after"): past it the
  queue switches from calibration tracks to unsure, conflicts and old labels.
- Show the model's guess in the player after calibration (On).
- Re-ask old tracks blind (rises as the library fills in).
- **Rekordbox Export**
  - Write the key into rekordbox's key field ("8A"): `[rekordbox] write_tonality`.
  - Start the comment with key and energy ("8A - Energy 6"); the rest of the
    comment, /* tags */ included, is kept: `[rekordbox] write_comment`.
  - No setting for how old the collection export may be: past 7 days
    (`warn_days`) the status row warns, and it always shows the export's age.
- **Cue points**
  - Write cue points (`[cues] write`). They go only on tracks with no cues or
    with generated ones; a track with any cue you set yourself is never touched.
  - Hot cues (`[cues] lead`), two presets:
    - "At each change · A–D" = `[0]`;
    - "16 + 8 bars before · A–H" = `[16, 8]`.
  - Loops (`[cues] loops`): slots A–H, each a cue or a loop. Click a slot to
    set its length in bars. Slots the chosen preset doesn't use are dimmed.
  - Memory cue on every hot cue and the first downbeat (`[cues] memory`).
- Key and energy you correct in the player are saved with Confirm in `labels.db`
  (`key_energy`, schema v5, the first the Python prototype can't open) and
  win over the analysis in the library and in every export.
- Not in settings yet:
  - `misleading_cost` and `rating_misleading_cost`. These could become
    "guess often ↔ only when sure" sliders.
  - `cues.retrain_after`.
  - `[audio]` and `[model]` never go in settings: they are pinned and part of
    the cache key.

## Open

- Should the table rows show cover art too? (The player does.)
- Should the Export button warn or block while the collection export is old?
- Should settings later drop "Start" and move the counts to stats?
- Should the collection player fold to a slim bar when no track is open?
