extends VBoxContainer
## One flat table like rekordbox: click a header to sort, click a row to open it in the player.
## The same table for the reviewed tracks and for the whole library.

const UI := preload("res://ui/ui.gd")

signal activated(t: Dictionary)
signal checked_changed(n: int)

## column -> [header, width]; width 0 takes the rest.
const COLS := {
	"check": ["", 16], "ring": ["", 12], "colour": ["Colour", 84], "title": ["Track", 0],
	"artist": ["Artist", 160], "energy": ["Energy", 104], "rating": ["Rating", 72], "bpm": ["BPM", 44],
	"key": ["Key", 40], "plays": ["Plays", 40], "last": ["Last played", 88], "added": ["Added", 64],
	"flag": ["", 110],
}

var cols: Array
var rows: Array[Dictionary] = []
var sort_key := "colour"
var sort_desc := false
var selected := -1
var checked := {}
var _header: HBoxContainer
var _list: VBoxContainer
var _scroll: ScrollContainer
var _panels := {}


func _init(c: Array) -> void:
	cols = c
	add_theme_constant_override("separation", 0)
	_header = UI.row(12)
	add_child(UI.margin(_header, 8, 0, 8, 4))
	add_child(UI.hline())
	_scroll = ScrollContainer.new()
	_scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	_scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_list = UI.col(0)
	_list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_scroll.add_child(_list)
	add_child(_scroll)
	size_flags_vertical = Control.SIZE_EXPAND_FILL


func set_rows(r: Array[Dictionary]) -> void:
	rows = r.duplicate()
	_sort()
	_build()


func sort_by(k: String) -> void:
	sort_desc = not sort_desc if sort_key == k else false
	sort_key = k
	_sort()
	_build()


func _value(t: Dictionary, k: String) -> Variant:
	if k == "title" or k == "artist":
		return str(t[k]).to_lower()
	if k == "last" or k == "added" or k == "key":
		return str(t[k])
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


func _build() -> void:
	for c in _header.get_children():
		c.queue_free()
	for k: String in cols:
		var h: String = COLS[k][0]
		if k == sort_key and h != "":
			h += " ▼" if sort_desc else " ▲"
		var l := _cell(UI.caps(h), k)
		if h != "":
			UI.clickable(l, sort_by.bind(k))
		_header.add_child(l)
	for c in _list.get_children():
		c.queue_free()
	_panels.clear()
	for t in rows:
		var p := _row(t)
		_panels[t.id] = p
		_list.add_child(p)


func _cell(c: Control, k: String) -> Control:
	var w: int = COLS[k][1]
	if w == 0:
		c.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		c.custom_minimum_size.x = 60
	else:
		c.custom_minimum_size.x = w
	var l := c as Label
	if l:
		l.clip_text = true
		l.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	return c


func _row(t: Dictionary) -> PanelContainer:
	var p := PanelContainer.new()
	p.custom_minimum_size.y = 24
	var r := UI.row(12)
	for k: String in cols:
		r.add_child(_cell(_content(t, k), k))
	p.add_child(UI.margin(r, 8, 0, 8, 0))
	_style(p, t)
	UI.clickable(p, func() -> void:
		select(t.id)
		activated.emit(t))
	return p


func _content(t: Dictionary, k: String) -> Control:
	var bold: bool = t.id == selected
	match k:
		"check":
			var cb := CheckBox.new()
			cb.button_pressed = checked.has(t.id)
			cb.focus_mode = Control.FOCUS_NONE
			cb.toggled.connect(func(on: bool) -> void:
				if on:
					checked[t.id] = true
				else:
					checked.erase(t.id)
				_style(_panels[t.id], t)
				checked_changed.emit(checked.size()))
			return cb
		"ring":
			return UI.Ring.new(t.trust)
		"colour":
			var cd: Dictionary = UI.COLOURS[t.colour]
			return UI.row(6, [UI.rect(cd.c, 8, 16), UI.text(cd.name, 12)])
		"title":
			return UI.text(t.title, 12, UI.TEXT, "bold" if bold else "sans")
		"artist":
			return UI.text(t.artist, 12, UI.MUTED)
		"energy":
			return UI.row(8, [UI.Bar.new(64, t.energy / 10.0), UI.text(str(t.energy), 12, UI.TEXT, "mono")])
		"rating":
			return UI.text("★".repeat(t.rating), 11, UI.TEXT, "mono")
		"flag":
			var f: String = "▶ in the player" if bold else str(t.flag)
			return UI.caps(f, UI.MUTED if f == "from library" else UI.LIME)
		_:
			var l := UI.text(str(t[k]), 12, UI.MUTED if k in ["last", "added"] else UI.TEXT, "mono")
			if k in ["bpm", "plays"]:
				l.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
			return l


func _style(p: PanelContainer, t: Dictionary) -> void:
	var s := StyleBoxFlat.new()
	s.border_color = Color("#34353a")
	s.border_width_bottom = 1
	s.bg_color = Color.TRANSPARENT
	if t.id == selected:
		s.bg_color = UI.SEL
		s.border_color = UI.LIME
		s.set_border_width_all(1)
	elif t.flag == "just reviewed":
		s.bg_color = UI.SEL
	elif checked.has(t.id):
		s.bg_color = Color("#2f3036")
	p.add_theme_stylebox_override("panel", s)


func select(id: int) -> void:
	var old := selected
	selected = id
	for i: int in [old, id]:
		if _panels.has(i):
			var t := _find(i)
			var p: PanelContainer = _panels[i]
			var idx := p.get_index()
			p.queue_free()
			var np := _row(t)
			_panels[i] = np
			_list.add_child(np)
			_list.move_child(np, idx)
	if _panels.has(id):
		_scroll.ensure_control_visible.call_deferred(_panels[id])


func _find(id: int) -> Dictionary:
	for t in rows:
		if t.id == id:
			return t
	return {}


## Step the selection up or down and open that row.
func move(d: int) -> void:
	if rows.is_empty():
		return
	var i := 0
	for j in rows.size():
		if rows[j].id == selected:
			i = clampi(j + d, 0, rows.size() - 1)
	select(rows[i].id)
	activated.emit(rows[i])


func checked_rows() -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	for t in rows:
		if checked.has(t.id):
			out.append(t)
	return out


func clear_checked() -> void:
	checked.clear()
	_build()
	checked_changed.emit(0)
