extends VBoxContainer
## How well the model knows your taste, in the open from the first label.

const UI := preload("res://ui/ui.gd")

## The learning curve's lines: [key, name, colour]. Colour is hue and tone together, so it
## sits lowest; hue and tone show which half of a wrong colour it misses.
## Each UI colour's palette id, as the core counts them.
var _ids: Array[int] = []
## The chart's unticked lines, for this session only: the colours start off, as each
## stands on few held-back labels and together they are noisy.
var _off := {}

const LINES := [
	["colour", "colour", Color("#c8f13c")],
	["hue", "hue", Color("#9fb8ff")],
	["tone", "tone", Color("#ffb07a")],
	["stars", "stars", Color("#e8e8ea")],
]
const SELF := Color("#a0a1a8")

var state
var _body: VBoxContainer
var _measured_at := -1
var _count: Label


func _init(s) -> void:
	state = s
	for i in UI.COLOURS.size():
		_ids.append(state.PALETTE.find(UI.COLOURS[i].name) + 1)
		_off[UI.COLOURS[i].name.to_lower()] = true
	add_theme_constant_override("separation", 0)
	_body = UI.col(14)
	var sc := ScrollContainer.new()
	sc.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	sc.size_flags_vertical = Control.SIZE_EXPAND_FILL
	var m := UI.margin(_body, 16, 12, 16, 16)
	m.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	sc.add_child(m)
	var head := UI.section(UI.PANEL, 16, 0)
	head.custom_minimum_size.y = 40
	_count = UI.text("", 11, UI.MUTED, "mono")
	head.add_child(UI.row(8, [UI.text("Stats", 13, UI.TEXT, "semi"), UI.spacer(), _count]))
	add_child(head)
	add_child(sc)
	# measuring trains the model several times: only when you look, and labels changed
	visibility_changed.connect(_measure)
	state.changed.connect(func() -> void:
		if is_visible_in_tree():
			_measure()
			refresh())
	refresh()


func _measure() -> void:
	if not is_visible_in_tree() or not state.loaded or _measured_at == state.labels:
		return
	_measured_at = state.labels
	state.load_stats(func(_s: Variant) -> void: refresh())


func refresh() -> void:
	for c in _body.get_children():
		c.queue_free()
	var st: Dictionary = state.stats
	if st.is_empty():
		_body.add_child(UI.text("Measuring…", 12, UI.MUTED))
		return
	var now: Dictionary = st.now
	_count.text = "%s labels" % UI.thousands(int(st.labels))
	# the last curve measured stays up, dimmed, while the one for your labels now is measured
	_body.add_child(UI.row(8, [UI.caps("Right vs labels trained on"),
		UI.text("" if st.fresh else "measuring…", 11, UI.MUTED)]))
	var off := _off
	# the four summary lines, then one per colour in the label colours (thinner): each
	# [key, name, colour, width, points]
	var all: Array = []
	for l: Array in LINES:
		var pts: Array[Vector2] = []
		for p: Dictionary in st.curve:
			if p.get(l[0]) != null:
				pts.append(Vector2(p.n, float(p[l[0]]) * 100))
		all.append([l[0], l[1], l[2], 2.5, pts])
	# past self: how often a colour you gave again stayed, counted up to each point; dashed,
	# about as high as the colour line can be expected to get
	var past: Array[Vector2] = []
	for p: Array in st.get("past", []):
		past.append(Vector2(p[0], float(p[1]) * 100))
	all.append(["self", "past self", SELF, 1.5, past, true])
	for i in UI.COLOURS.size():
		var id: int = _ids[i]
		var pts: Array[Vector2] = []
		for p: Dictionary in st.curve:
			for e: Array in p.get("by_colour", []):
				if int(e[0]) == id:
					pts.append(Vector2(p.n, float(e[1]) * 100))
		all.append([UI.COLOURS[i].name.to_lower(), UI.COLOURS[i].name, UI.COLOURS[i].c, 1.5, pts])
	var lc := LearningCurve.new(all.filter(func(l: Array) -> bool: return not off.has(l[0])))
	if not st.fresh:
		lc.modulate.a = 0.5
	_body.add_child(lc)
	# a box per line, its colour beside it: the legend, and what to show (remembered);
	# the summary lines on one row, the colours, from few held-back labels each, on the next
	var rows := [UI.row(16), UI.row(16)]
	for j in all.size():
		var l: Array = all[j]
		var cb := CheckBox.new()
		cb.button_pressed = not off.has(l[0])
		cb.focus_mode = Control.FOCUS_NONE
		cb.add_theme_font_size_override("font_size", 12)
		cb.text = l[1]
		var k: String = l[0]
		cb.toggled.connect(func(b: bool) -> void:
			if b:
				_off.erase(k)
			else:
				_off[k] = true
			refresh.call_deferred())
		rows[1 if _id_of(l[0]) >= 0 else 0].add_child(UI.row(2, [cb, UI.rect(l[2], 14, 3)]))
	_body.add_child(rows[0])
	_body.add_child(rows[1])
	_body.add_child(UI.caps("Trust"))
	var counts := [0, 0, 0, 0]
	for t: Dictionary in state.library:
		if t.trust > 0:
			counts[int(t.trust) - 1] += 1
	var bar := UI.row(2)
	var names := ["guessed", "existing", "confirmed", "played"]
	var colours := [Color("#4a4b50"), UI.AMBER, Color("#8fb02a"), UI.LIME]
	for i in [3, 2, 1, 0]:
		var r := UI.rect(colours[i], 0, 10)
		r.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		r.size_flags_stretch_ratio = maxf(counts[i], 1)
		bar.add_child(r)
	_body.add_child(bar)
	var legend := UI.row(14)
	for i in [3, 2, 1, 0]:
		legend.add_child(UI.row(5, [UI.rect(colours[i], 8, 8), UI.text("%s %d" % [names[i], counts[i]], 11, UI.MUTED)]))
	legend.add_child(UI.text("· no label %d" % (state.library.size() - counts.reduce(func(a: int, b: int) -> int: return a + b)), 11, UI.MUTED))
	_body.add_child(legend)
	# the library by colour, guesses included, in the label colours: what a set has to pick from
	_body.add_child(UI.caps("Colours"))
	var by := []
	by.resize(UI.COLOURS.size())
	by.fill(0)
	for t: Dictionary in state.library:
		if int(t.colour) >= 0:
			by[int(t.colour)] += 1
	var cbar := UI.row(2)
	var clegend := UI.row(14)
	for i in UI.COLOURS.size():
		var r := UI.rect(UI.COLOURS[i].c, 0, 10)
		r.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		r.size_flags_stretch_ratio = maxf(by[i], 1)
		cbar.add_child(r)
		clegend.add_child(UI.row(5, [UI.rect(UI.COLOURS[i].c, 8, 8), UI.text("%s %d" % [UI.COLOURS[i].name, by[i]], 11, UI.MUTED)]))
	clegend.add_child(UI.text("· no colour %d" % (state.library.size() - by.reduce(func(a: int, b: int) -> int: return a + b, 0)), 11, UI.MUTED))
	_body.add_child(cbar)
	_body.add_child(clegend)
	# misses: tracks you gave one colour that it guessed as another, worst first, by the
	# share of that colour's held-back labels; one per row, so the rank reads top down
	_body.add_child(UI.caps("Misses · you said → it guessed"))
	var held := {}
	for c: Array in now.get("by_colour", []):
		held[int(c[0])] = int(c[2])
	var ms: Array = now.confused.map(func(m: Array) -> Array:
		return [int(m[0]), int(m[1]), int(m[2]), maxi(held.get(int(m[0]), 0), int(m[2]))])
	ms.sort_custom(func(x: Array, y: Array) -> bool: return x[2] * y[3] > y[2] * x[3] or x[2] * y[3] == y[2] * x[3] and x[2] > y[2])
	var list := UI.col(6)
	for m: Array in ms.slice(0, 4):
		var a: Dictionary = UI.COLOURS[_ui(m[0])]
		var b: Dictionary = UI.COLOURS[_ui(m[1])]
		var p := UI.panel(UI.PANEL, 4, 10, 6)
		p.add_child(UI.row(6, [UI.rect(a.c, 8, 16), UI.text(a.name, 12), UI.text("→", 12, UI.MUTED), UI.rect(b.c, 8, 16), UI.text(b.name, 12),
			UI.spacer(), UI.text("%d / %d · %d%%" % [m[2], m[3], roundi(100.0 * m[2] / maxi(m[3], 1))], 11, UI.MUTED, "mono")]))
		list.add_child(p)
	if ms.is_empty():
		_body.add_child(UI.text("nothing measured yet", 12, UI.MUTED))
	_body.add_child(list)


func _id_of(key: String) -> int:
	for i in UI.COLOURS.size():
		if UI.COLOURS[i].name.to_lower() == key:
			return _ids[i]
	return -1


func _ui(palette_id: int) -> int:
	return state._ui_of.get(palette_id, 0)


## Share right against labels trained on, measured on your own held-back labels: one line
## per [key, name, colour, width, points, dashed?], each named at its end with where it is now. Always 0–100%, so
## ticking a line on or off moves nothing; labels on a log scale, as each point doubles them.
class LearningCurve extends Control:
	const LABEL_W := 84.0
	const LEFT := 36.0
	const FIRST := 20.0  # labels at the left edge; the first point is at 25
	var lines: Array = []
	var top := 1000.0
	var ticks: Array[int] = []

	func _init(l: Array) -> void:
		lines = l
		var n := 0.0
		for line: Array in l:
			for v: Vector2 in line[4]:
				n = maxf(n, v.x)
		# the right edge a little past now (at least 1,000), dotted lines at 100, 500, 1000…
		top = maxf(n * 1.5, 1000)
		var t := 100
		while t <= top:
			ticks.append(t)
			t = t * 5 if str(t)[0] == "1" else t * 2
		custom_minimum_size = Vector2(0, 480)

	func _x(v: float) -> float:
		return LEFT + log(maxf(v, FIRST) / FIRST) / log(top / FIRST) * (size.x - LEFT - 10 - LABEL_W)

	func _y(v: float) -> float:
		return size.y - 20 - v / 100.0 * (size.y - 30)

	func _draw() -> void:
		var f := ThemeDB.fallback_font
		var muted := Color("#a0a1a8")
		var right := size.x - 10 - LABEL_W
		draw_rect(Rect2(Vector2.ZERO, size), UI.BG)
		for v in range(0, 101, 10):
			draw_line(Vector2(LEFT, _y(v)), Vector2(right, _y(v)), Color("#2f3036"))
			if v % 20 == 0:
				draw_string(f, Vector2(4, _y(v) + 4), "%d%%" % v, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, muted)
		for g: int in ticks:
			draw_dashed_line(Vector2(_x(g), 6), Vector2(_x(g), size.y - 20), Color("#5a5b62"), 1, 4)
			draw_string(f, Vector2(_x(g) - 40, size.y - 4), UI.thousands(g), HORIZONTAL_ALIGNMENT_CENTER, 80, 10, muted)
		var drawn: Array = []
		# thin colour lines first, the summary lines over them
		var order := lines.duplicate()
		order.sort_custom(func(a: Array, b: Array) -> bool: return a[3] < b[3])
		for line: Array in order:
			var ps: Array[Vector2] = line[4]
			if ps.size() < 2:
				continue
			var pts := PackedVector2Array()
			for p: Vector2 in ps:
				pts.append(Vector2(_x(p.x), _y(clampf(p.y, 0, 100))))
			if line.size() > 5 and line[5]:
				for i in pts.size() - 1:
					draw_dashed_line(pts[i], pts[i + 1], line[2], line[3], 6)
			else:
				draw_polyline(pts, line[2], line[3], true)
			for p in pts:
				draw_circle(p, line[3] + 0.5, line[2])
			drawn.append([pts[pts.size() - 1], "%s  %d%%" % [line[1], roundi(ps[-1].y)], line[2]])
		if drawn.is_empty():
			draw_string(f, Vector2(40, size.y / 2), "needs more labels to draw" if not lines.is_empty() else "no line ticked",
				HORIZONTAL_ALIGNMENT_LEFT, -1, 11, muted)
			return
		# names at the line ends, pushed apart where lines end close together
		drawn.sort_custom(func(a: Array, b: Array) -> bool: return a[0].y < b[0].y)
		var last := -INF
		for d: Array in drawn:
			var y := maxf(d[0].y + 4, last + 14)
			last = y
			draw_string(f, Vector2(d[0].x + 10, y), d[1], HORIZONTAL_ALIGNMENT_LEFT, -1, 11, d[2])
		# above the last points: how many labels the model now is trained on
		draw_string(f, Vector2(drawn[0][0].x - 120, drawn[0][0].y - 12), "now, %d trained on" % roundi(_n()),
			HORIZONTAL_ALIGNMENT_RIGHT, 120, 10, muted)

	func _n() -> float:
		var n := 0.0
		for line: Array in lines:
			if not line[4].is_empty():
				n = maxf(n, line[4][-1].x)
		return n
