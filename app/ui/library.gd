extends VBoxContainer
## The whole collection: the same player docked on top, filters, and the same flat table.
## Confirm puts a track into the reviewed table, so it goes out with the next Export.

const UI := preload("res://ui/ui.gd")
const Player := preload("res://ui/player.gd")
const Table := preload("res://ui/table.gd")

const FILTERS := ["All", "Played", "Pending", "Thin trust"]

var state
var app
var player: Player
var table: Table
var search: LineEdit
var filter := "All"
var _chips: HBoxContainer
var _selection: PanelContainer
var _sel_count: Label
var _dirty := true


func _init(s, a) -> void:
	state = s
	app = a
	add_theme_constant_override("separation", 0)
	search = LineEdit.new()
	search.placeholder_text = "Search your tracks"
	search.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	search.add_theme_stylebox_override("normal", StyleBoxEmpty.new())
	search.add_theme_stylebox_override("focus", StyleBoxEmpty.new())
	search.add_theme_font_size_override("font_size", 13)
	search.text_changed.connect(func(_t: String) -> void: _fill())
	search.gui_input.connect(func(e: InputEvent) -> void:
		if e is InputEventKey and e.pressed and e.keycode == KEY_ESCAPE:
			search.release_focus())
	var head := UI.section(UI.PANEL, 16, 4)
	head.custom_minimum_size.y = 40
	head.add_child(UI.row(8, [UI.kbd("/"), search, UI.button("Columns", "", false, func() -> void: pass)]))
	add_child(head)
	player = Player.new(UI.DARK, state)
	player.next.connect(_on_next)
	player.skip.connect(func() -> void: table.move(1))
	add_child(player)
	_chips = UI.row(6)
	var cs := UI.section(UI.PANEL, 16, 8)
	cs.add_child(_chips)
	add_child(cs)
	_selection = UI.section(Color("#2f3036"), 16, 6)
	_sel_count = UI.text("", 12, UI.TEXT, "mono")
	_selection.add_child(UI.row(10, [_sel_count,
		UI.button("Add to queue", "⌥Q", true, _queue_checked), UI.button("Clear", "", false, func() -> void: table.clear_checked()),
		UI.spacer(), UI.kbd("⌥C"), UI.text("colour", 11, UI.MUTED), UI.kbd("⌥E"), UI.text("energy", 11, UI.MUTED),
		UI.kbd("⌥R"), UI.text("rating", 11, UI.MUTED), UI.kbd("⌥B"), UI.text("bpm", 11, UI.MUTED), UI.kbd("⌥T"), UI.text("trust", 11, UI.MUTED),
		UI.kbd("↑↓"), UI.text("move", 11, UI.MUTED)]))
	_selection.visible = false
	add_child(_selection)
	table = Table.new(["now", "check", "ring", "colour", "title", "artist", "energy", "rating", "bpm", "key", "plays", "labelled", "added"])
	table.activated.connect(func(t: Dictionary) -> void: player.show_track(t, "library"))
	search.text_submitted.connect(func(_t: String) -> void: search.release_focus())
	table.checked_changed.connect(func(n: int) -> void:
		_selection.visible = n > 0
		_sel_count.text = "%d selected" % n)
	var tm := UI.margin(table, 16, 6, 16, 10)
	tm.size_flags_vertical = Control.SIZE_EXPAND_FILL
	add_child(tm)
	player.clear("Pick a track below, or step through with ↑↓")
	# 15,000 rows: refilter only when this screen is in view
	state.changed.connect(func() -> void:
		_dirty = true
		if is_visible_in_tree():
			_fill())
	visibility_changed.connect(func() -> void:
		if is_visible_in_tree() and _dirty:
			_fill())


func _fill() -> void:
	_dirty = false
	search.placeholder_text = "Search %s tracks" % UI.thousands(state.library.size())
	for c in _chips.get_children():
		c.queue_free()
	var pending := 0
	for t in state.library:
		if t.trust <= 1:
			pending += 1
	for f: String in FILTERS:
		var label := "Pending %d" % pending if f == "Pending" else f
		_chips.add_child(UI.chip(label, f == filter, func() -> void:
			filter = f
			_fill()))
	_chips.add_child(UI.rect(Color("#4a4b50"), 1, 20))
	var dots := UI.row(2)
	for cd: Dictionary in UI.COLOURS:
		dots.add_child(UI.rect(cd.c, 6, 10))
	var allc := UI.panel(UI.LINE, 3, 9, 4)
	allc.add_child(UI.row(5, [dots, UI.text("all colours", 11)]))
	_chips.add_child(allc)
	for f in ["Energy 1–10", "Rating any", "BPM any", "Batch any"]:
		_chips.add_child(UI.chip(f, false))
	var q := search.text.to_lower()
	var out: Array[Dictionary] = []
	for t: Dictionary in state.library:
		if q != "" and not (t.title.to_lower().contains(q) or t.artist.to_lower().contains(q)):
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
	_chips.add_child(UI.spacer())
	_chips.add_child(UI.text("%d tracks" % out.size(), 11, UI.MUTED, "mono"))
	table.set_rows(out)


func _on_next(colour: int, rating: int, energy: int, key: String) -> void:
	# the next row as it is now: a new colour re-sorts the saved track elsewhere
	var i := table.rows.find(player.track)
	var nxt: Dictionary = table.rows[mini(i + 1, table.rows.size() - 1)]
	state.decide(player.track, colour, rating, "library", energy, key)
	table.select(nxt.path)
	player.show_track(nxt, "library")


func _queue_checked() -> void:
	state.add_to_queue(table.checked_rows())
	table.clear_checked()


func handle_key(k: InputEventKey) -> bool:
	if k.keycode == KEY_SLASH:
		search.grab_focus()
		return true
	if k.alt_pressed:
		if k.keycode == KEY_Q:
			_queue_checked()
			return true
		var sorts := {KEY_C: "colour", KEY_E: "energy", KEY_R: "rating", KEY_B: "bpm", KEY_T: "ring"}
		if sorts.has(k.keycode):
			table.sort_by(sorts[k.keycode])
			return true
		return false
	if k.physical_keycode == KEY_UP or k.physical_keycode == KEY_DOWN:
		table.move(-1 if k.physical_keycode == KEY_UP else 1)
		return true
	return player.handle_key(k)
