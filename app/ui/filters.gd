extends HBoxContainer
## Search and filter chips over a table, the same on the library and the vibe screen's
## export list: "/" search field, All / Played / Pending / Thin trust, then colours, energy,
## rating and BPM, each a popup as the player's energy and key pickers (pick as many values as
## you like, it stays open; empty is any), then how many rows show and anything `tail` holds.
## `changed` when any of it changes; the screen then calls `apply` with its rows.

const UI := preload("res://ui/ui.gd")
const FILTERS := ["All", "Played", "Pending", "Thin trust"]

signal changed

var search: LineEdit
var filter := "All"
## Colours: -1 is no colour; rating: 0 is none; BPM in tens, 70 holding all below, 180 all above.
var _picked := {"colours": {}, "energy": {}, "rating": {}, "bpm": {}}
var _pop: PopupPanel
var _pop_at := Vector2i.ZERO
var _openers := {}  # filter name: its chip
var _chips: HBoxContainer
var _count: Label
## Only the colour, energy, rating and BPM chips, over the fast track's queue: they narrow
## what the model's guess is asked about, so no search, trust filters or count.
var guess_only := false


func _init(tail: Array = [], guess := false) -> void:
	guess_only = guess
	add_theme_constant_override("separation", 8)
	search = LineEdit.new()
	search.placeholder_text = "Search tracks"
	search.custom_minimum_size.x = 260
	# looks like a field: a darker box with a border, lime while typing (it read as a label)
	for st: String in ["normal", "focus"]:
		var b := StyleBoxFlat.new()
		b.bg_color = UI.BG
		b.border_color = UI.LIME if st == "focus" else Color("#4a4b50")
		b.set_border_width_all(1)
		b.set_corner_radius_all(4)
		search.add_theme_stylebox_override(st, UI.pad(b, 10, 5))
	search.add_theme_color_override("font_placeholder_color", UI.MUTED)
	search.add_theme_color_override("caret_color", UI.LIME)
	search.add_theme_font_size_override("font_size", 13)
	search.text_changed.connect(func(_t: String) -> void: changed.emit())
	search.text_submitted.connect(func(_t: String) -> void: search.release_focus())
	search.gui_input.connect(func(e: InputEvent) -> void:
		if e is InputEventKey and e.pressed and e.keycode == KEY_ESCAPE:
			search.release_focus())
	_chips = UI.row(6)
	_chips.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_count = UI.text("", 11, UI.MUTED, "mono")
	var head: Array = [] if guess_only else [UI.kbd("/"), search, UI.rect(Color("#4a4b50"), 1, 20)]
	for c: Control in head + [_chips] + ([] if guess_only else [_count]):
		add_child(c)
	for c: Control in tail:
		add_child(UI.margin(c, 8, 0, 0, 0))
	if guess_only:
		rebuild(0)


## What is picked, for the core: colours as UI indexes (the caller maps them), the rest as is.
func picked() -> Dictionary:
	var out := {}
	for n: String in _picked:
		out[n] = (_picked[n] as Dictionary).keys()
	return out


## The rows that pass, with the chips rebuilt for them ("Pending N" counts `rows`).
func apply(rows: Array) -> Array[Dictionary]:
	var pending := 0
	for t: Dictionary in rows:
		if t.trust <= 1:
			pending += 1
	rebuild(pending)
	var q := search.text.to_lower()
	var out: Array[Dictionary] = []
	for t: Dictionary in rows:
		if q != "" and not (t.title.to_lower().contains(q) or t.artist.to_lower().contains(q)):
			continue
		if not (_has("colours", int(t.colour)) and _has("energy", int(t.energy)) and _has("rating", int(t.rating))
				and _has("bpm", clampi(int(t.bpm) / 10 * 10, 70, 180))):
			continue
		match filter:
			"Played":
				if t.plays == 0:
					continue
			"Pending":
				if t.trust > 1:
					continue
			"Thin trust":
				if t.trust > 2:
					continue
		out.append(t)
	_count.text = "%s tracks" % UI.thousands(out.size())
	return out


## The chips as things stand; `pending` for the "Pending N" chip.
func rebuild(pending: int) -> void:
	for c in _chips.get_children():
		c.queue_free()
	if not guess_only:
		for f: String in FILTERS:
			var label := "Pending %s" % UI.thousands(pending) if f == "Pending" else f
			_chips.add_child(UI.chip(label, f == filter, func() -> void:
				filter = f
				changed.emit()))
		_chips.add_child(UI.rect(Color("#4a4b50"), 1, 20))
	# colours: a dot for each one shown
	var dots := UI.row(2)
	var cs: Dictionary = _picked.colours
	for i in UI.COLOURS.size():
		if cs.is_empty() or cs.has(i):
			dots.add_child(UI.rect(UI.COLOURS[i].c, 6, 10))
	# one colour by its name, more by how many
	var one: int = cs.keys()[0] if cs.size() == 1 else -2
	var cname := "all colours"
	if one == -1:
		cname = "no colour"
	elif one >= 0:
		cname = UI.COLOURS[one].name
	elif not cs.is_empty():
		cname = "%d colours" % cs.size()
	var allc := UI.chip(cname, not cs.is_empty())
	var lab: Label = allc.get_child(0)
	allc.remove_child(lab)
	allc.add_child(UI.row(5, [dots, lab]))
	_openers = {}
	for n: String in ["colours", "energy", "rating", "bpm"]:
		var chip: Control = allc if n == "colours" else UI.chip(_label(n), not (_picked[n] as Dictionary).is_empty())
		UI.clickable(chip, _pick.bind(n, chip))
		_chips.add_child(chip)
		_openers[n] = chip


## Open a filter's popup, as a click on its chip does (for screenshots).
func open_filter(n: String) -> void:
	_pick(n, _openers[n])


func _has(n: String, v: int) -> bool:
	var d: Dictionary = _picked[n]
	return d.is_empty() or d.has(v)


## The chip's text: what is picked, runs joined ("Energy 4–6, 9"); BPM runs as their span.
func _label(n: String) -> String:
	var name: String = {"energy": "Energy", "rating": "Rating", "bpm": "BPM"}[n]
	var vs: Array = (_picked[n] as Dictionary).keys()
	if vs.is_empty():
		return name + " any"
	vs.sort()
	var step := 10 if n == "bpm" else 1
	var parts: Array[String] = []
	var i := 0
	while i < vs.size():
		var j := i
		while j + 1 < vs.size() and vs[j + 1] == vs[j] + step:
			j += 1
		if n == "bpm":
			parts.append("%d–%d" % [vs[i], vs[j] + 9])
		elif n == "rating" and vs[i] == 0 and i == j:
			parts.append("none")
		else:
			parts.append(str(vs[i]) if i == j else "%d–%d" % [vs[i], vs[j]])
		i = j + 1
	return name + " " + ", ".join(parts)


## The popup for filter `n`: any, then every value, each on or off.
func _pick(n: String, at: Control) -> void:
	var items: Array = [[-99, "any" if n != "colours" else "all"]]
	match n:
		"colours":
			for i in UI.COLOURS.size():
				items.append([i, UI.COLOURS[i].name, UI.COLOURS[i].c])
			if not guess_only:
				items.append([-1, "no colour"])
		"energy":
			for e in range(1, 11):
				items.append([e, str(e)])
		"rating":
			items.append([0, "none"])
			for r in range(1, 6):
				items.append([r, "★".repeat(r)])
		"bpm":
			for b in range(70, 190, 10):
				items.append([b, str(b)])
	var d: Dictionary = _picked[n]
	_popup(at, items, {"colours": 5, "energy": 11, "rating": 7, "bpm": 13}[n],
		func(v: int) -> bool: return d.is_empty() if v == -99 else d.has(v),
		func(v: int) -> bool:
			if v == -99:
				d.clear()
				return false
			if not d.erase(v):
				d[v] = true
			return true)


## A grid of [value, label, swatch?] under a chip, as the player's pickers: the current lit.
## `pick` says whether to stay open; the screen refilters either way.
func _popup(at: Control, items: Array, cols: int, lit: Callable, pick: Callable) -> void:
	if _pop:
		_pop.queue_free()
	if at:
		_pop_at = Vector2i(at.get_screen_position()) + Vector2i(0, int(at.size.y) + 6)
	_pop = PopupPanel.new()
	_pop.add_theme_stylebox_override("panel", UI.pad(UI.style(UI.DARK, 6, Color("#55565c"), 1), 10, 10))
	var g := GridContainer.new()
	g.columns = cols
	g.add_theme_constant_override("h_separation", 4)
	g.add_theme_constant_override("v_separation", 4)
	for it: Array in items:
		var on: bool = lit.call(it[0])
		var p := UI.panel(UI.LIME if on else UI.CARD, 3, 8, 0)
		p.custom_minimum_size = Vector2(36, 28)
		var r := UI.row(6)
		r.alignment = BoxContainer.ALIGNMENT_CENTER
		if it.size() > 2:
			r.add_child(UI.rect(it[2], 4, 12))
		r.add_child(UI.text(it[1], 12, UI.BG if on else UI.TEXT, "mono"))
		p.add_child(r)
		UI.clickable(p, func() -> void:
			var keep: bool = pick.call(it[0])
			changed.emit()
			if keep:
				_popup.call_deferred(null, items, cols, lit, pick)
			else:
				_pop.hide())
		g.add_child(p)
	_pop.add_child(g)
	add_child(_pop)
	_pop.popup(Rect2i(_pop_at, Vector2i.ZERO))
