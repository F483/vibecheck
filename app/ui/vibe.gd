extends VBoxContainer
## The vibe screen: player and its controls on top, as in the library; then queue →
## the export list: search and filters (the library's, Export at the end), then its table.

const UI := preload("res://ui/ui.gd")
const Player := preload("res://ui/player.gd")
const Table := preload("res://ui/table.gd")
const Filters := preload("res://ui/filters.gd")

var state
var app
var player: Player
var table: Table
var relisten := false
var _queue: VBoxContainer
## Search and filters over the export list, the library's row, with Export at its end.
var filters: Filters
var _shown := -1
var _status := ""
## The player: lent to the library as it is, so changing screen moves nothing on it.
var top: VBoxContainer
var _controls: CenterContainer


func _init(s, a, p) -> void:
	state = s
	app = a
	add_theme_constant_override("separation", 0)
	top = UI.col(0)
	add_child(top)
	# shared with the library: Confirm and Skip belong to the screen in view
	player = p
	player.next.connect(func(c: int, r: int, e: int, k: String) -> void:
		if is_visible_in_tree():
			_on_next(c, r, e, k))
	player.skip.connect(func() -> void:
		if is_visible_in_tree():
			_on_skip())
	top.add_child(player)
	# the player's main controls on their own strip under it, as in the library
	_controls = CenterContainer.new()
	_controls.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_controls.add_child(player.take_controls())
	var cs := UI.section(UI.PANEL, 16, 8)
	cs.add_child(_controls)
	add_child(cs)
	# the queue only here, under the controls, so the player sits at the top on both screens
	_queue = UI.col(8)
	var q := UI.section(UI.DARK)
	q.add_child(_queue)
	add_child(q)
	filters = Filters.new([UI.button("Export →", "X", true, Callable(app, "export_batch"), UI.OUTPUT)])
	filters.changed.connect(_fill)
	var r := UI.section(UI.PANEL, 16, 8)
	r.add_child(filters)
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


## The player comes here when this screen is shown. A track from the library stays, playing,
## until Confirm or Skip, which go on with the queue.
func dock() -> void:
	lend(self, _controls, "vibe")
	player.ahead = _ahead
	if player.mode == "library":
		relisten = false
	refresh()


## The player into `parent`, first, and its controls into `controls`.
func lend(parent: Control, controls: Control, w: String) -> void:
	if top.get_parent() != parent:
		top.reparent(parent, false)
	parent.move_child(top, 0)
	player.dock(controls, w)


func refresh() -> void:
	# a new queue: load its head, unless you are relistening to a reviewed track, or the
	# player holds one from the library; not while the library has the player
	var here := top.get_parent() == self
	if here and not relisten and player.mode != "library" and (state.queue.is_empty() or state.queue[0] != player.track):
		_head()
	elif here and relisten:
		player.fill_guess(player.track)
	_build_queue()
	_fill()


## The export list into the table, through the filters.
func _fill() -> void:
	# the # column: the order tracks joined the list, as in a rekordbox playlist; over the
	# whole list, so a filter keeps each track's number
	var by_join: Array = state.reviewed.duplicate()
	# ties broken by title, as the table breaks them, so sorting by # gives 1, 2, 3…
	by_join.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
		var ja := int(a.get("joined", 0))
		var jb := int(b.get("joined", 0))
		return str(a.title) < str(b.title) if ja == jb else ja < jb)
	for i in by_join.size():
		by_join[i].num = i + 1
	table.spinning = state.guessing > 0
	table.set_rows(filters.apply(state.reviewed))


func _build_queue() -> void:
	for c in _queue.get_children():
		c.queue_free()
	# the queue holds the next 200; the pool is everything that could be asked
	_queue.add_child(UI.row(8, [UI.caps("Queue"), UI.text(str(state.queue.size()), 12, UI.TEXT, "mono"),
		UI.text("of %s" % UI.thousands(maxi(state.pool, state.queue.size())), 12, UI.MUTED, "mono"),
		UI.text("ordered by what teaches the model most", 11, UI.MUTED), UI.text(_status, 11, UI.LIME, "mono")]))
	var cards := UI.row(8)
	var n := _fit()
	_shown = n
	for i in n:
		cards.add_child(_card(state.queue[i], i == 0, not relisten and state.queue[i].path == player.track.get("path")))
	# held in a clipping box: how many cards show follows the width (_fit), so they don't
	# count towards how narrow the window can get
	var hold := Control.new()
	hold.custom_minimum_size.y = 64
	hold.clip_contents = true
	hold.mouse_filter = Control.MOUSE_FILTER_IGNORE
	cards.set_anchors_preset(Control.PRESET_FULL_RECT)
	hold.add_child(cards)
	_queue.add_child(hold)


## Outlined while it is in the player, not while you relisten to the export list.
func _card(t: Dictionary, first: bool, playing: bool) -> Control:
	var p := UI.panel(UI.CARD, 4, 7, 7)
	if playing:
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
	# into the player, on the library screen too
	UI.clickable(p, func() -> void:
		state.queue.erase(t)
		state.queue.push_front(t)
		relisten = false
		_head()
		refresh())
	return p


## How many queue cards fit.
## Measured from the window, not from this screen, so cards never push the layout wider.
func _fit() -> int:
	var w := get_viewport_rect().size.x - 64 if is_inside_tree() else 1216.0
	var room := w - 32  # margins
	return mini(maxi(int((room + 8) / 192), 1), state.queue.size())


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
	var t := player.track
	state.decide(t, colour, rating, "vibe", energy, key, player.shown, player.shown_stars, player.card())
	if relisten:
		open_row(nxt)
	elif player.track == t:
		_head()  # a library track: refresh leaves those, so on with the queue here


## What the player loads next: the next row when relistening (Confirm and Skip both go
## there), else the queue's second (after Confirm, and after Requeue).
func _ahead() -> Array:
	if relisten:
		return [_next_row()]
	return [state.queue[1]] if state.queue.size() > 1 and state.queue[0] == player.track else []


func _next_row() -> Dictionary:
	var at := table.rows.find(player.track)
	return table.rows[at + 1] if at >= 0 and at + 1 < table.rows.size() else {}


func _on_skip() -> void:
	if player.mode == "dropped" and not _next_dropped().is_empty():
		open_row(_next_dropped())
	elif player.mode == "library":
		_head()
		_build_queue()
	elif relisten:
		# next row as sorted; after the last, back to the queue, as Confirm
		var nxt := _next_row()
		if nxt.is_empty():
			_head()
			_build_queue()
		else:
			open_row(nxt)
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
	var was := relisten
	relisten = true
	if t.erase("fresh"):
		table.queue_redraw()
	player.show_track(t, "dropped" if t.get("dropped", false) else "relisten")
	if not was:
		_build_queue()  # the queue's track left the player


## Open a table row in the player, as a click on it does.
func open_row(t: Dictionary) -> void:
	table.select(t.path)
	_on_row(t)


## Whether a point in the window is on the reviewed strip or its table, for a drop.
func is_table(at: Vector2) -> bool:
	return at.y >= (filters.get_parent() as Control).get_global_rect().position.y


func handle_key(k: InputEventKey) -> bool:
	if k.keycode == KEY_SLASH:
		filters.search.grab_focus()
		return true
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
