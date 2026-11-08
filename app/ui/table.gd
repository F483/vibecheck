extends Control
## One flat table like rekordbox: click a header to sort, click a row to open it in the player.
## The same table for the reviewed tracks and for the whole library. Drawn, not built from
## nodes, so a library of tens of thousands of tracks scrolls as fast as a batch of ten.

const UI := preload("res://ui/ui.gd")

signal activated(t: Dictionary)
signal checked_changed(n: int)

## column -> [header, width]; width 0 takes the rest.
const COLS := {
	"now": ["", 8], "num": ["#", 28], "check": ["", 16], "ring": ["", 12], "colour": ["Colour", 84], "title": ["Track", 0],
	"artist": ["Artist", 160], "energy": ["Energy", 104], "rating": ["Rating", 72], "bpm": ["BPM", 44],
	"key": ["Key", 40], "plays": ["Plays", 40], "labelled": ["Labelled", 80], "added": ["Added", 80],
}
const ROW := 24.0
const HEAD := 28.0
const GAP := 12.0
const PAD := 8.0

var cols: Array
var rows: Array[Dictionary] = []
var sort_key := "colour"
var sort_desc := false
var selected := ""
var checked := {}
var scroll := 0.0
var _bar: VScrollBar
var _x := {}  # column -> [x, width]
var _fit := {}  # [text, width] -> text that fits
var _sans: Font
var _bold: Font
var _mono: Font
var _trust: PopupPanel

## The trust levels, as the ring shows them: [name, what you did, what it counts for].
const TRUST := [
	[4, "played", "you played it after the label was written", "counts most"],
	[3, "confirmed", "you confirmed it here, or changed it in rekordbox", "counts a lot"],
	[2, "existing", "it was there before vibecheck", "counts a little"],
	[1, "guessed", "the model guessed it, nothing since", "counts least"],
]
const TRUST_NEXT := {
	4: "This label counts fully.",
	3: "Play it in a set to make it count most.",
	2: "Play it, or confirm it here, to make it count more.",
	1: "Play it, or correct its colour, to make it count more.",
	0: "No label yet.",
}


func _init(c: Array) -> void:
	cols = c
	size_flags_vertical = Control.SIZE_EXPAND_FILL
	size_flags_horizontal = Control.SIZE_EXPAND_FILL
	clip_contents = true
	focus_mode = Control.FOCUS_NONE
	_sans = UI.font("sans")
	_bold = UI.font("bold")
	_mono = UI.font("mono")
	_bar = VScrollBar.new()
	_bar.set_anchors_and_offsets_preset(Control.PRESET_RIGHT_WIDE)
	_bar.offset_left = -8
	_bar.offset_top = HEAD
	_bar.value_changed.connect(func(v: float) -> void:
		scroll = v
		queue_redraw())
	add_child(_bar)
	resized.connect(_layout)


func set_rows(r: Array[Dictionary]) -> void:
	rows = r.duplicate()
	_sort()
	_layout()


func sort_by(k: String) -> void:
	sort_desc = not sort_desc if sort_key == k else false
	sort_key = k
	_sort()
	queue_redraw()


func _value(t: Dictionary, k: String) -> Variant:
	match k:
		"title", "artist":
			return str(t[k]).to_lower()
		"added", "key":
			return str(t[k])
		"colour":
			return t.colour if t.colour >= 0 else 99
		"ring":
			return int(t.trust)
		"num":
			return int(t.get("joined", 0))
	return t.get(k, 0)


func _sort() -> void:
	var k := sort_key
	var d := sort_desc
	rows.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
		var va: Variant = _value(a, k)
		var vb: Variant = _value(b, k)
		if va == vb:
			return str(a.title) < str(b.title)
		return (va > vb) if d else (va < vb))


func _layout() -> void:
	var fixed := 0.0
	for k: String in cols:
		fixed += COLS[k][1]
	var rest := maxf(size.x - 2 * PAD - 8 - fixed - GAP * (cols.size() - 1), 60)
	var x := PAD
	for k: String in cols:
		var w: float = COLS[k][1] if COLS[k][1] > 0 else rest
		_x[k] = [x, w]
		x += w + GAP
	var view := size.y - HEAD
	_bar.max_value = rows.size() * ROW
	_bar.page = view
	_bar.visible = rows.size() * ROW > view
	scroll = clampf(scroll, 0, maxf(rows.size() * ROW - view, 0))
	_bar.set_value_no_signal(scroll)
	queue_redraw()


## The text cut to fit `w`, with an ellipsis; remembered, since rows redraw on every scroll.
func _clip(s: String, w: float, f: Font, fs: int) -> String:
	var key := "%s|%d|%d" % [s, w, fs]
	if _fit.has(key):
		return _fit[key]
	var out := s
	if f.get_string_size(s, HORIZONTAL_ALIGNMENT_LEFT, -1, fs).x > w:
		var lo := 0
		var hi := s.length()
		while lo < hi:
			var m := (lo + hi + 1) / 2
			if f.get_string_size(s.left(m) + "…", HORIZONTAL_ALIGNMENT_LEFT, -1, fs).x <= w:
				lo = m
			else:
				hi = m - 1
		out = s.left(lo) + "…"
	if _fit.size() > 20000:
		_fit.clear()
	_fit[key] = out
	return out


func _text(s: String, k: String, y: float, f: Font, c: Color, fs := 12, right := false) -> void:
	var x: float = _x[k][0]
	var w: float = _x[k][1]
	var t := _clip(s, w, f, fs)
	if right:
		x += w - f.get_string_size(t, HORIZONTAL_ALIGNMENT_LEFT, -1, fs).x
	draw_string(f, Vector2(x, y + ROW / 2 + fs * 0.36), t, HORIZONTAL_ALIGNMENT_LEFT, -1, fs, c)


func _draw() -> void:
	var first := int(scroll / ROW)
	var last := mini(rows.size(), first + int((size.y - HEAD) / ROW) + 2)
	for i in range(first, last):
		var t := rows[i]
		var y := HEAD + i * ROW - scroll
		var sel: bool = t.path == selected
		var r := Rect2(0, y, size.x - 8, ROW)
		if sel:
			draw_rect(r, UI.SEL)
			draw_rect(r.grow(-0.5), UI.LIME, false, 1)
		elif t.get("fresh", false):
			# just dropped in, not opened yet: find them wherever the sort put them
			draw_rect(r, Color(UI.LIME, 0.08))
			draw_rect(Rect2(0, y, 3, ROW), UI.LIME)
		elif t.flag == "just reviewed":
			draw_rect(r, UI.SEL)
		elif checked.has(t.path):
			draw_rect(r, Color("#2f3036"))
		draw_line(Vector2(0, y + ROW - 0.5), Vector2(size.x - 8, y + ROW - 0.5), Color("#34353a"))
		for k: String in cols:
			_cell(t, k, y, sel)
	# the header last, over any row scrolled under it
	draw_rect(Rect2(0, 0, size.x, HEAD), UI.DARK)
	for k: String in cols:
		var h: String = COLS[k][0].to_upper()
		if k == "ring":
			# trust has no word, just a ring, like its cells
			draw_arc(Vector2(_x[k][0] + 6, HEAD / 2), 4.5, 0, TAU, 24, UI.MUTED, 1.5, true)
			if k == sort_key:
				draw_string(_mono, Vector2(_x[k][0] + 13, HEAD / 2 + 4), "▼" if sort_desc else "▲", HORIZONTAL_ALIGNMENT_LEFT, -1, 10, UI.MUTED)
			continue
		if k == sort_key and h != "":
			h = (("▼ " if sort_desc else "▲ ") + h) if k == "num" else h + (" ▼" if sort_desc else " ▲")
		var hx: float = _x[k][0]
		if k == "num":
			# over its numbers, which are right-aligned
			hx += _x[k][1] - _mono.get_string_size(h, HORIZONTAL_ALIGNMENT_LEFT, -1, 10).x
		draw_string(_mono, Vector2(hx, HEAD / 2 + 4), h, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, UI.MUTED)
	draw_line(Vector2(0, HEAD - 0.5), Vector2(size.x, HEAD - 0.5), UI.LINE)


func _cell(t: Dictionary, k: String, y: float, sel: bool) -> void:
	var x: float = _x[k][0]
	var mid := y + ROW / 2
	match k:
		"check":
			var b := Rect2(x + 1, mid - 6, 12, 12)
			if checked.has(t.path):
				draw_rect(b, UI.LIME)
				draw_polyline([b.position + Vector2(3, 6), b.position + Vector2(5, 9), b.position + Vector2(9, 3)], UI.BG, 1.5, true)
			else:
				draw_rect(b, Color("#6a6b70"), false, 1)
		"ring":
			var c := Vector2(x + 6, mid)
			var lv: int = t.trust
			var col := UI.LIME if lv >= 3 else (UI.AMBER if lv == 2 else UI.MUTED)
			draw_arc(c, 4.5, 0, TAU, 24, Color("#4a4b50"), 2.0, true)
			if lv > 0:
				draw_arc(c, 4.5, -PI / 2, -PI / 2 + TAU * lv / 4.0, 24, col, 2.0, true)
		"colour":
			if t.colour >= 0:
				var cd: Dictionary = UI.COLOURS[t.colour]
				draw_rect(Rect2(x, mid - 8, 8, 16), cd.c)
				draw_string(_sans, Vector2(x + 14, mid + 4.3), cd.name, HORIZONTAL_ALIGNMENT_LEFT, -1, 12, UI.TEXT)
			else:
				draw_string(_sans, Vector2(x + 14, mid + 4.3), "—", HORIZONTAL_ALIGNMENT_LEFT, -1, 12, UI.MUTED)
		"title":
			_text(t.title, k, y, _bold if sel else _sans, UI.TEXT)
		"artist":
			_text(t.artist, k, y, _sans, UI.MUTED)
		"energy":
			if t.energy > 0:
				draw_rect(Rect2(x, mid - 2, 64, 4), UI.LINE)
				draw_rect(Rect2(x, mid - 2, 64 * t.energy / 10.0, 4), Color("#d6d7db"))
				draw_string(_mono, Vector2(x + 72, mid + 4.3), str(t.energy), HORIZONTAL_ALIGNMENT_LEFT, -1, 12, UI.TEXT)
		"rating":
			_text("★".repeat(maxi(t.rating, 0)), k, y, _mono, UI.TEXT, 11)
		"bpm":
			_text("%.0f" % t.bpm if t.bpm > 0 else "", k, y, _mono, UI.TEXT, 12, true)
		"plays":
			_text(str(t.plays), k, y, _mono, UI.TEXT, 12, true)
		"num":
			_text(str(t.get("num", "")), k, y, _mono, UI.MUTED, 12, true)
		"now":
			# the row in the player
			if sel:
				draw_colored_polygon([Vector2(x, mid - 4.5), Vector2(x + 7, mid), Vector2(x, mid + 4.5)], UI.LIME)
		_:
			_text(str(t[k]), k, y, _mono, UI.MUTED if k in ["labelled", "added"] else UI.TEXT)


func _gui_input(e: InputEvent) -> void:
	if e is InputEventMouseButton and e.pressed:
		match e.button_index:
			MOUSE_BUTTON_WHEEL_UP, MOUSE_BUTTON_WHEEL_DOWN:
				_scroll_to(scroll + (-1 if e.button_index == MOUSE_BUTTON_WHEEL_UP else 1) * ROW * 3)
			MOUSE_BUTTON_LEFT:
				_click(e.position)
		accept_event()
	elif e is InputEventPanGesture:
		_scroll_to(scroll + e.delta.y * ROW)
		accept_event()


func _scroll_to(v: float) -> void:
	scroll = clampf(v, 0, maxf(rows.size() * ROW - (size.y - HEAD), 0))
	_bar.set_value_no_signal(scroll)
	queue_redraw()


func _click(p: Vector2) -> void:
	if p.y < HEAD:
		for k: String in cols:
			if (COLS[k][0] != "" or k == "ring") and p.x >= _x[k][0] and p.x < _x[k][0] + _x[k][1] + GAP:
				sort_by(k)
		return
	var i := int((p.y - HEAD + scroll) / ROW)
	if i < 0 or i >= rows.size():
		return
	var t := rows[i]
	if cols.has("ring") and p.x >= _x["ring"][0] - GAP / 2 and p.x < _x["ring"][0] + _x["ring"][1] + GAP / 2:
		_trust_popup(t, Vector2i(get_screen_position() + p))
		return
	if cols.has("check") and p.x < _x["check"][0] + _x["check"][1] + GAP / 2:
		if checked.has(t.path):
			checked.erase(t.path)
		else:
			checked[t.path] = true
		checked_changed.emit(checked.size())
		queue_redraw()
		return
	select(t.path)
	activated.emit(t)


## What the ring means, with this track's level lit and what would raise it.
func _trust_popup(t: Dictionary, at: Vector2i) -> void:
	if _trust:
		_trust.queue_free()
	_trust = PopupPanel.new()
	_trust.add_theme_stylebox_override("panel", UI.pad(UI.style(UI.DARK, 6, Color("#55565c"), 1), 12, 10))
	var lv: int = int(t.trust)
	var v := UI.col(6, [UI.caps("Trust: how much the model learns from this label")])
	var g := GridContainer.new()
	g.columns = 4
	g.add_theme_constant_override("h_separation", 10)
	g.add_theme_constant_override("v_separation", 4)
	for row: Array in TRUST:
		var on: bool = row[0] == lv
		g.add_child(UI.Ring.new(row[0]))
		g.add_child(UI.text(row[1], 12, UI.LIME if on else UI.TEXT, "bold" if on else "sans"))
		g.add_child(UI.text(row[2], 12, UI.TEXT if on else UI.MUTED))
		g.add_child(UI.text(row[3], 11, UI.LIME if on else UI.MUTED, "mono"))
	v.add_child(g)
	var nxt: String = TRUST_NEXT.get(lv, "")
	v.add_child(UI.margin(UI.text(nxt, 12, UI.TEXT), 0, 4, 0, 0))
	_trust.add_child(v)
	add_child(_trust)
	_trust.popup(Rect2i(at + Vector2i(8, 8), Vector2i.ZERO))


func select(path: String) -> void:
	selected = path
	for i in rows.size():
		if rows[i].path == path:
			var y := i * ROW
			var view := size.y - HEAD
			if y < scroll:
				_scroll_to(y)
			elif y + ROW > scroll + view:
				_scroll_to(y + ROW - view)
			break
	queue_redraw()


## Step the selection up or down and open that row.
func move(d: int) -> void:
	if rows.is_empty():
		return
	var i := -1
	for j in rows.size():
		if rows[j].path == selected:
			i = j
	i = clampi(i + d, 0, rows.size() - 1)
	select(rows[i].path)
	activated.emit(rows[i])


func checked_rows() -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	for t in rows:
		if checked.has(t.path):
			out.append(t)
	return out


func clear_checked() -> void:
	checked.clear()
	queue_redraw()
	checked_changed.emit(0)
