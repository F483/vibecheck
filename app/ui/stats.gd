extends VBoxContainer
## How well the model knows your taste, in the open from the first label.

const UI := preload("res://ui/ui.gd")

var state
var _body: VBoxContainer
var _measured_at := -1


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
	_body.modulate.a = 0.5
	state.load_stats(func(_s: Variant) -> void:
		_body.modulate.a = 1.0
		refresh())


func _pct(r: Array) -> String:
	return "%d–%d%%" % [roundi(r[0] * 100), roundi(r[1] * 100)]


func refresh() -> void:
	for c in _body.get_children():
		c.queue_free()
	var st: Dictionary = state.stats
	if st.is_empty():
		_body.add_child(UI.text("Measuring…", 12, UI.MUTED))
		return
	var now: Dictionary = st.now
	var again := int(st.again)
	var tiles := UI.row(8)
	for t: Array in [
			["Colour right", _pct(now.colour) if now.colour_n > 0 else "—", "on %d held-back labels" % now.colour_n],
			["Stars right", _pct(now.stars) if now.stars_n > 0 else "—", "on %d held-back labels" % now.stars_n],
			["Past self", "%d%%" % roundi(100.0 * st.agreed / again) if again > 0 else "—", "same colour on %d tracks asked twice" % again],
			["Labels", str(int(st.labels)), "yours, that it has listened to"]]:
		var p := UI.panel(UI.DARK, 4, 12, 10)
		p.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		p.add_child(UI.col(2, [UI.caps(t[0]), UI.text(t[1], 22, UI.LIME, "mono"), UI.text(t[2], 10, UI.MUTED)]))
		tiles.add_child(p)
	_body.add_child(tiles)
	_body.add_child(UI.caps("Mixed up most"))
	var grid := GridContainer.new()
	grid.columns = 2
	grid.add_theme_constant_override("h_separation", 8)
	grid.add_theme_constant_override("v_separation", 6)
	for m: Array in now.confused:
		var a: Dictionary = UI.COLOURS[_ui(int(m[0]))]
		var b: Dictionary = UI.COLOURS[_ui(int(m[1]))]
		var p := UI.panel(UI.DARK, 4, 10, 6)
		p.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		p.add_child(UI.row(6, [UI.rect(a.c, 8, 16), UI.rect(b.c, 8, 16), UI.text("%s taken for %s" % [a.name, b.name], 12),
			UI.spacer(), UI.text("%d%%" % roundi(100.0 * m[2] / maxf(now.colour_n, 1)), 11, UI.MUTED, "mono")]))
		grid.add_child(p)
	if now.confused.is_empty():
		_body.add_child(UI.text("nothing measured yet", 12, UI.MUTED))
	_body.add_child(grid)
	_body.add_child(UI.caps("Colour right vs labels trained on"))
	var pts: Array[Vector2] = []
	for p: Array in st.curve:
		pts.append(Vector2(p[0], p[1] * 100))
	_body.add_child(LearningCurve.new(pts))
	_body.add_child(UI.caps("Trust"))
	var counts := [0, 0, 0, 0]
	for t: Dictionary in state.library:
		if t.trust > 0:
			counts[int(t.trust) - 1] += 1
	var bar := UI.row(2)
	var names := ["untouched guess", "existing", "by hand", "played"]
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


func _ui(palette_id: int) -> int:
	return state._ui_of.get(palette_id, 0)


## Colour right against labels trained on, measured on your own held-back labels.
class LearningCurve extends Control:
	var points: Array[Vector2] = []
	var top := 1000.0

	func _init(p: Array[Vector2]) -> void:
		points = p
		for v in p:
			top = maxf(top, v.x * 1.1)
		custom_minimum_size = Vector2(0, 160)

	func _x(v: float) -> float:
		return 30 + v / top * (size.x - 40)

	func _y(v: float) -> float:
		return size.y - 20 - (v - 30) / 60.0 * (size.y - 30)

	func _draw() -> void:
		var f := ThemeDB.fallback_font
		draw_rect(Rect2(Vector2.ZERO, size), Color("#222327"))
		for v in [40, 50, 60, 70, 80]:
			draw_line(Vector2(30, _y(v)), Vector2(size.x - 10, _y(v)), Color("#2f3036"))
			draw_string(f, Vector2(4, _y(v) + 4), "%d%%" % v, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color("#a0a1a8"))
		for g in [100, 300]:
			draw_dashed_line(Vector2(_x(g), 6), Vector2(_x(g), size.y - 20), Color("#5a5b62"), 1, 4)
			draw_string(f, Vector2(_x(g) + 4, 16), "goal %d" % g if g == 300 else "min %d" % g, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color("#a0a1a8"))
		if points.size() < 2:
			draw_string(f, Vector2(40, size.y / 2), "needs more labels to draw", HORIZONTAL_ALIGNMENT_LEFT, -1, 11, Color("#a0a1a8"))
			return
		var pts := PackedVector2Array()
		for p: Vector2 in points:
			pts.append(Vector2(_x(p.x), _y(clampf(p.y, 30, 90))))
		draw_polyline(pts, Color("#c8f13c"), 2, true)
		var you := pts[pts.size() - 1]
		draw_circle(you, 5, Color("#c8f13c"))
		draw_string(f, you + Vector2(-110, -10), "now, %d trained on" % int(points[-1].x), HORIZONTAL_ALIGNMENT_LEFT, -1, 11, Color("#e8e8ea"))
