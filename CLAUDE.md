# vibecheck

- The Python prototype is archived in `prototype.zip` (its tracked files as
  of 2026-11-12); nothing maintains it. Work goes into `core/` (Rust), `ext/`
  and `app/` (Godot).
- Database schema v5 and later are Rust-only; the prototype stayed at v4.
- `docs/user-flows.md` is the plan for how the app is used; check it before
  building UI or changing how labels are trusted.
- UI design: `docs/ui/decisions.md` (why) and `docs/ui/workspace.html`/`.png`
  (mockup), mirrored from https://claude.ai/artifact/VGEERe7xK3zbRnQ2Etn7FS.
  Update all three with every mockup change, and follow them when building
  the Godot UI.
