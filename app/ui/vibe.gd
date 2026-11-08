extends VBoxContainer
## The vibe screen, top to bottom as the flow: queue → model → player → reviewed → table.

const UI := preload("res://ui/ui.gd")
const Player := preload("res://ui/player.gd")
const Table := preload("res://ui/table.gd")

var state
var app
var player: Player
var table: Table
var relisten := false
var _queue: VBoxContainer
var _model: HBoxContainer
var _reviewed: HBoxContainer
var _shown := -1
var _status := ""


func _init(s, a) -> void:
	state = s
	app = a
	add_theme_constant_override("separation", 0)
	_queue = UI.col(8)
	var q := UI.section(UI.DARK)
	q.add_child(_queue)
	add_child(q)
	_model = UI.row(12)
	var m := UI.section(UI.PANEL, 16, 8)
	m.add_child(_model)
	add_child(m)
	player = Player.new(UI.DARK, state)
	player.next.connect(_on_next)
	player.skip.connect(_on_skip)
	add_child(player)
	_reviewed = UI.row(10)
	var r := UI.section(UI.PANEL, 16, 8)
	r.add_child(_reviewed)
	add_child(r)
	table = Table.new(["now", "num", "ring", "colour", "title", "artist", "energy", "rating", "bpm", "key"])
	table.activated.connect(_on_row)
	var tm := UI.margin(table, 16, 6, 16, 10)
	tm.size_flags_vertical = Control.SIZE_EXPAND_FILL
	add_child(tm)
	state.changed.connect(refresh)
	state.status.connect(func(t: String) -> void:
		_status = t
		_build_queue())
	resized.connect(func() -> void:
		if _fit() != _shown:
			_build_queue())
	refresh()


func refresh() -> void:
	# a new queue: load its head, unless you are relistening to a reviewed track
	if not relisten and (state.queue.is_empty() or state.queue[0] != player.track):
		_head()
	elif relisten:
		player.fill_guess(player.track)
	_build_queue()
	_build_model()
	for c in _reviewed.get_children():
		c.queue_free()
	var parts: Array = [UI.text("↓", 13, UI.MUTED), UI.caps("Export list"), UI.text(str(state.reviewed.size()), 12, UI.TEXT, "mono")]
	for c: Control in parts + [UI.spacer(), UI.button("Export →", "X", true, Callable(app, "export_batch"))]:
		_reviewed.add_child(c)
	var rows: Array[Dictionary] = []
	rows.append_array(state.reviewed)
	# the # column: the order tracks joined the list, as in a rekordbox playlist
	var by_join := rows.duplicate()
	# ties broken by title, as the table breaks them, so sorting by # gives 1, 2, 3…
	by_join.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
		var ja := int(a.get("joined", 0))
		var jb := int(b.get("joined", 0))
		return str(a.title) < str(b.title) if ja == jb else ja < jb)
	for i in by_join.size():
		by_join[i].num = i + 1
	table.set_rows(rows)


func _build_queue() -> void:
	for c in _queue.get_children():
		c.queue_free()
	_queue.add_child(UI.row(8, [UI.caps("Queue"), UI.text(str(state.queue.size()), 12, UI.TEXT, "mono"),
		UI.text("ordered by what teaches the model most", 11, UI.MUTED), UI.spacer(), UI.text(_status, 11, UI.LIME, "mono")]))
	var cards := UI.row(8)
	var n := _fit()
	_shown = n
	for i in n:
		cards.add_child(_card(state.queue[i], i == 0))
	if state.queue.size() > n:
		var more := UI.panel(UI.DARK, 4)
		var s: StyleBoxFlat = more.get_theme_stylebox("panel")
		s.border_color = Color("#4a4b50")
		s.set_border_width_all(1)
		more.custom_minimum_size = Vector2(72, 64)
		# the queue holds the next 200; the pool is everything that could be asked
		var l := UI.text("+%d" % (state.queue.size() - n), 12, UI.MUTED)
		var of := UI.text("of %s" % UI.thousands(maxi(state.pool, state.queue.size())), 10, UI.MUTED, "mono")
		for x: Label in [l, of]:
			x.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		var c := UI.col(0, [l, of])
		c.alignment = BoxContainer.ALIGNMENT_CENTER
		more.add_child(c)
		cards.add_child(more)
	cards.add_child(UI.spacer())
	cards.add_child(UI.text("←", 16, UI.MUTED))
	cards.add_child(_widget())
	_queue.add_child(cards)


func _card(t: Dictionary, first: bool) -> Control:
	var p := UI.panel(UI.CARD, 4, 7, 7)
	if first:
		var s: StyleBoxFlat = p.get_theme_stylebox("panel")
		s.border_color = UI.LIME
		s.set_border_width_all(1)
	p.custom_minimum_size = Vector2(184, 64)
	var info := UI.col(1, [UI.tag("next" if first else str(t.tag)), UI.text(t.title, 12, UI.TEXT, "bold"),
		UI.text(t.artist, 11, UI.MUTED)])
	for l in info.get_children():
		if l is Label:
			l.clip_text = true
			l.custom_minimum_size.x = 110
	p.add_child(UI.row(8, [UI.cover(t, 48, state.cover(t)), info]))
	UI.clickable(p, func() -> void:
		state.queue.erase(t)
		state.queue.push_front(t)
		relisten = false
		refresh())
	return p


## How many queue cards fit next to the +N box and the rekordbox widget.
## Measured from the window, not from this screen, so cards never push the layout wider.
func _fit() -> int:
	var w := get_viewport_rect().size.x - 64 if is_inside_tree() else 1216.0
	var room := w - 32 - 72 - 16 - 300 - 32  # margins, +N box, arrow, widget, gaps
	return mini(maxi(int((room + 8) / 192), 1), state.queue.size())


func _ago(days: int) -> String:
	match days:
		-1:
			return "not yet"
		0:
			return "today"
		1:
			return "yesterday"
	return "%d d ago" % days


## Where the queue's tracks come from: rekordbox, and how fresh its export is.
func _widget() -> Control:
	var old: bool = state.export_old()
	var p := UI.panel(UI.PANEL, 4, 9, 6)
	var s: StyleBoxFlat = p.get_theme_stylebox("panel")
	s.border_color = Color("#6a5420") if old else UI.LINE
	s.set_border_width_all(1)
	p.custom_minimum_size = Vector2(300, 64)
	var sync := UI.panel(UI.LINE, 4, 6, 1)
	sync.add_child(UI.row(4, [UI.text("Sync", 11, UI.TEXT, "bold"), UI.kbd("⌥S")]))
	UI.clickable(sync, Callable(app, "sync_now"))
	var v := UI.col(2, [
		UI.row(6, [UI.caps("rekordbox"), UI.rect(UI.LIME, 6, 6), UI.text("watching " + str(state.settings.xml).get_file(), 11, UI.MUTED),
			UI.spacer(), sync]),
		UI.row(6, [UI.text("collection exported " + _ago(state.export_age_days()), 12, UI.AMBER if old else UI.TEXT), UI.spacer(),
			UI.text("+%d queued" % state.queued_by_sync, 12, UI.LIME, "mono")]),
	])
	if old:
		v.add_child(UI.text("⚠ old: export it again in rekordbox before Export", 10, UI.AMBER))
	p.add_child(v)
	return p


func _build_model() -> void:
	for c in _model.get_children():
		c.queue_free()
	var acc: Vector2i = state.accuracy()
	var th: int = state.settings.threshold
	var parts: Array = [UI.text("↓", 13, UI.MUTED), UI.caps("Model")]
	if not state.loaded:
		parts += [UI.text("reading your labels…", 12, UI.MUTED)]
	elif state.calibrating():
		parts += [UI.tag("early calibration"), UI.text("learning your taste, guess hidden (blind)", 12, UI.MUTED),
			UI.Bar.new(160, float(state.labels) / th, UI.LIME), UI.text("%d / %d labels" % [state.labels, th], 12, UI.TEXT, "mono")]
	else:
		parts += [UI.tag("mixing in"), UI.text("unsure tracks, conflicts and old labels join the queue", 12, UI.MUTED),
			UI.text("%d labels" % state.labels, 12, UI.TEXT, "mono")]
	parts += [UI.spacer(), UI.text("model guesses right", 12, UI.MUTED)]
	if state.measured > 0:
		parts += [UI.text("%d–%d%%" % [acc.x, acc.y], 14, UI.LIME, "mono"),
			UI.text("on %d held-back labels" % state.measured, 11, UI.MUTED)]
	else:
		parts += [UI.text("not measured yet", 12, UI.MUTED)]
	parts += [UI.text("↓", 13, UI.MUTED)]
	for c: Control in parts:
		_model.add_child(c)


## Load the front of the queue into the player, blind while calibrating.
func _head() -> void:
	relisten = false
	table.select("")
	if state.queue.is_empty():
		player.clear("Reading your collection…" if not state.loaded else "Queue is empty: sync from rekordbox or add tracks from the library")
		return
	var t: Dictionary = state.queue[0]
	if t.tag == "from rekordbox":
		# dragged in to be edited: show what it has
		player.show_track(t, "queue", int(t.colour), int(t.rating))
		return
	var blind: bool = state.calibrating() or not state.settings.show_guess
	player.show_track(t, "queue", -1 if blind else t.guess, -1 if blind else t.guess_stars)


## Save: a track from the queue moves on to the next in the queue; one from the list to the
## next row in the list, as it is sorted now (a new colour re-sorts the saved one elsewhere),
## so a sorted list can be worked through top to bottom. After the last row, the queue.
func _on_next(colour: int, rating: int, energy: int, key: String) -> void:
	var nxt := _next_row() if relisten else {}
	relisten = not nxt.is_empty()
	state.decide(player.track, colour, rating, "vibe", energy, key)
	if relisten:
		open_row(nxt)


func _next_row() -> Dictionary:
	var at := table.rows.find(player.track)
	return table.rows[at + 1] if at >= 0 and at + 1 < table.rows.size() else {}


func _on_skip() -> void:
	if player.mode == "dropped" and not _next_dropped().is_empty():
		open_row(_next_dropped())
	elif relisten:
		_head()
		_build_queue()
	elif not state.queue.is_empty():
		state.skip(state.queue[0])


## The dropped row after the one in the player, in the table's order, wrapping round.
func _next_dropped() -> Dictionary:
	var rows := table.rows.filter(func(t: Dictionary) -> bool: return t.get("dropped", false) and t != player.track)
	if rows.is_empty():
		return {}
	var at := table.rows.find(player.track)
	for t: Dictionary in rows:
		if table.rows.find(t) > at:
			return t
	return rows[0]


func _on_row(t: Dictionary) -> void:
	relisten = true
	if t.erase("fresh"):
		table.queue_redraw()
	player.show_track(t, "dropped" if t.get("dropped", false) else "relisten")


## Open a table row in the player, as a click on it does.
func open_row(t: Dictionary) -> void:
	table.select(t.path)
	_on_row(t)


## Whether a point in the window is on the reviewed strip or its table, for a drop.
func is_table(at: Vector2) -> bool:
	return at.y >= (_reviewed.get_parent() as Control).get_global_rect().position.y


func handle_key(k: InputEventKey) -> bool:
	if k.alt_pressed:
		var sorts := {KEY_C: "colour", KEY_E: "energy", KEY_R: "rating", KEY_B: "bpm", KEY_T: "ring"}
		if sorts.has(k.keycode):
			table.sort_by(sorts[k.keycode])
			return true
		return false
	if k.physical_keycode == KEY_UP or k.physical_keycode == KEY_DOWN:
		table.move(-1 if k.physical_keycode == KEY_UP else 1)
		return true
	return player.handle_key(k)
