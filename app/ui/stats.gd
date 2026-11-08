extends VBoxContainer
## How well the model knows your taste, in the open from the first label.

const UI := preload("res://ui/ui.gd")

var state
var _body: VBoxContainer


func _init(s) -> void:
	state = s
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
	head.add_child(UI.row(8, [UI.text("Stats", 13, UI.TEXT, "semi")]))
	add_child(head)
	add_child(sc)
	state.changed.connect(refresh)
	refresh()


func refresh() -> void:
	for c in _body.get_children():
		c.queue_free()
	var acc: Vector2i = state.accuracy()
	var tiles := UI.row(8)
	for t: Array in [["Colour right", "%d–%d%%" % [acc.x, acc.y]], ["Stars right", "72–78%"],
			["Past self", "84%"], ["Labels", str(state.labels)]]:
		var p := UI.panel(UI.DARK, 4, 12, 10)
		p.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		p.add_child(UI.col(2, [UI.caps(t[0]), UI.text(t[1], 22, UI.LIME, "mono")]))
		tiles.add_child(p)
	_body.add_child(tiles)
	_body.add_child(UI.caps("Mixed up most"))
	var grid := GridContainer.new()
	grid.columns = 2
	grid.add_theme_constant_override("h_separation", 8)
	grid.add_theme_constant_override("v_separation", 6)
	for pair: Array in [[2, 6, "9%"], [0, 1, "7%"], [7, 6, "6%"], [3, 4, "4%"]]:
		var p := UI.panel(UI.DARK, 4, 10, 6)
		p.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		var a: Dictionary = UI.COLOURS[pair[0]]
		var b: Dictionary = UI.COLOURS[pair[1]]
		p.add_child(UI.row(6, [UI.rect(a.c, 8, 16), UI.rect(b.c, 8, 16), UI.text("%s ↔ %s" % [a.name, b.name], 12),
			UI.spacer(), UI.text(pair[2], 11, UI.MUTED, "mono")]))
		grid.add_child(p)
	_body.add_child(grid)
	_body.add_child(UI.caps("Colour right vs labels"))
	var chart := LearningCurve.new(state.labels)
	_body.add_child(chart)
	_body.add_child(UI.caps("Trust"))
	var counts := [0, 0, 0, 0]
	for t: Dictionary in state.library:
		counts[int(t.trust) - 1] += 1
	var bar := UI.row(2)
	var names := ["untouched", "existing", "by hand", "played"]
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
	_body.add_child(legend)


## The learning curve measured on the reference collection, and where you are on it.
class LearningCurve extends Control:
	const POINTS := [Vector2(0, 40), Vector2(100, 56), Vector2(200, 62), Vector2(300, 65), Vector2(500, 68), Vector2(800, 69), Vector2(1200, 70)]
	var labels := 0

	func _init(n: int) -> void:
		labels = n
		custom_minimum_size = Vector2(0, 160)

	func _x(v: float) -> float:
		return 30 + v / 1200.0 * (size.x - 40)

	func _y(v: float) -> float:
		return size.y - 20 - (v - 35) / 40.0 * (size.y - 30)

	func _draw() -> void:
		var f := ThemeDB.fallback_font
		draw_rect(Rect2(Vector2.ZERO, size), Color("#222327"))
		for v in [40, 50, 60, 70]:
			draw_line(Vector2(30, _y(v)), Vector2(size.x - 10, _y(v)), Color("#2f3036"))
			draw_string(f, Vector2(4, _y(v) + 4), "%d%%" % v, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color("#a0a1a8"))
		var pts := PackedVector2Array()
		for p: Vector2 in POINTS:
			pts.append(Vector2(_x(p.x), _y(p.y)))
		draw_polyline(pts, Color("#c8f13c"), 2, true)
		for g in [100, 300]:
			draw_dashed_line(Vector2(_x(g), 6), Vector2(_x(g), size.y - 20), Color("#5a5b62"), 1, 4)
			draw_string(f, Vector2(_x(g) + 4, 16), "goal %d" % g if g == 300 else "min %d" % g, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color("#a0a1a8"))
		var you := Vector2(_x(minf(labels, 1200)), _y(_at(labels)))
		draw_circle(you, 5, Color("#c8f13c"))
		draw_string(f, you + Vector2(8, -6), "you, %d labels" % labels, HORIZONTAL_ALIGNMENT_LEFT, -1, 11, Color("#e8e8ea"))

	func _at(n: float) -> float:
		for i in POINTS.size() - 1:
			var a: Vector2 = POINTS[i]
			var b: Vector2 = POINTS[i + 1]
			if n <= b.x:
				return lerpf(a.y, b.y, (n - a.x) / (b.x - a.x))
		return 70.0
