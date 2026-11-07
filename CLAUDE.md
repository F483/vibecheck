# vibecheck

- `prototype/` (Python) is frozen: change nothing there unless asked. New work
  goes into `core/` (Rust), `ext/` and `app/` (Godot).
- Database schema v5 and later are Rust-only; the prototype stays at v4.
- `docs/user-flows.md` is the plan for how the app is used; check it before
  building UI or changing how labels are trusted.
