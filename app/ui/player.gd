extends PanelContainer
## The player: one track, its waveform, the colour keys and stars, and two buttons
## that keep their place and keys everywhere (⇧ENTER and ENTER). Used by vibe and library.

const UI := preload("res://ui/ui.gd")

signal next(colour: int, rating: int)
signal skip

const SECTION_NAMES := ["INTRO", "BUILD", "DROP", "BREAK", "DROP", "OUTRO"]
const SECTIONS := [0.0, 0.17, 0.37, 0.55, 0.67, 0.88]
const ENERGY := [0.25, 0.5, 0.95, 0.35, 0.95, 0.3]

## The two buttons and their tag, per context.
const MODES := {
	"queue": {"tag": "", "skip": "Skip track", "hint": ""},
	"relisten": {"tag": "relistening · from reviewed", "skip": "Requeue", "hint": ""},
	"library": {"tag": "relistening · from library", "skip": "Skip", "hint": "Next saves and adds to the next Export ·"},
}

var track: Dictionary = {}
var mode := "queue"
var colour := -1
var rating := 0
var playing := false
var pos := 0.0
var _undo: Array[Vector2i] = []

var _head: HBoxContainer
var _wave: Wave
var _keys: GridContainer
var _stars: HBoxContainer
var _actions: VBoxContainer


func _init(bg: Color) -> void:
	add_theme_stylebox_override("panel", UI.pad(_bg(bg), 16, 14))
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	_head = UI.row(10)
	_wave = Wave.new(UI.font("mono"))
	_wave.seek.connect(func(f: float) -> void:
		pos = f * _length()
		_tick())
	_keys = GridContainer.new()
	_keys.columns = 4
	_keys.add_theme_constant_override("h_separation", 6)
	_keys.add_theme_constant_override("v_separation", 6)
	_stars = UI.row(4)
	var help := UI.row(6, [UI.kbd("←"), UI.kbd("→"), UI.text("section", 11, UI.MUTED), UI.kbd("SPACE"),
		UI.text("play", 11, UI.MUTED), UI.kbd("1"), UI.text("–", 11, UI.MUTED), UI.kbd("6"),
		UI.text("jump", 11, UI.MUTED), UI.kbd("U"), UI.text("undo", 11, UI.MUTED)])
	_actions = UI.col(6)
	_actions.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	add_child(UI.col(10, [_head, _wave, UI.row(20, [_keys, UI.col(10, [_stars, help]), UI.spacer(), _actions])]))


func _bg(c: Color) -> StyleBoxFlat:
	var s := UI.style(c)
	s.border_color = UI.LINE
	s.border_width_bottom = 1
	return s


func show_track(t: Dictionary, m: String, guess := -1) -> void:
	track = t
	mode = m
	colour = guess if m == "queue" else int(t.colour)
	rating = 0 if m == "queue" and guess < 0 else int(t.rating)
	pos = 0.0
	_undo.clear()
	_wave.build(int(t.seed))
	_refresh()


func clear() -> void:
	track = {}
	playing = false
	for c in _head.get_children():
		c.queue_free()
	_head.add_child(UI.text("Queue is empty: sync from rekordbox or add tracks from the library", 14, UI.MUTED))


func _length() -> float:
	return float(track.get("length", 1.0))


func _refresh() -> void:
	for c in _head.get_children():
		c.queue_free()
	var cfg: Dictionary = MODES[mode]
	var tg: String = cfg.tag if cfg.tag != "" else str(track.tag)
	if tg != "":
		_head.add_child(UI.tag(tg))
	_head.add_child(UI.text(track.title, 22, UI.TEXT, "heavy"))
	_head.add_child(UI.text("%s · %s" % [track.artist, track.mix], 13, UI.MUTED))
	_head.add_child(UI.spacer())
	_head.add_child(UI.text("", 12, UI.MUTED, "mono"))
	_refresh_keys()
	_refresh_stars()
	for c in _actions.get_children():
		c.queue_free()
	_actions.add_child(UI.row(8, [UI.spacer(), UI.button(cfg.skip, "⇧ENTER", false, func() -> void: skip.emit()),
		UI.button("Next", "ENTER", true, _next)]))
	if cfg.hint != "":
		_actions.add_child(UI.row(4, [UI.spacer(), UI.text(cfg.hint, 11, UI.MUTED), UI.kbd("ESC"), UI.text("stop", 11, UI.MUTED)]))
	_tick()


func _refresh_keys() -> void:
	for c in _keys.get_children():
		c.queue_free()
	for i in 8:
		var cd: Dictionary = UI.COLOURS[i]
		var on := i == colour
		var p := UI.panel(cd.c if on else UI.CARD, 4, 10, 0)
		p.custom_minimum_size = Vector2(124, 44)
		var r := UI.row(8, [UI.kbd(OS.get_keycode_string(cd.key), on)])
		if not on:
			r.add_child(UI.rect(cd.c, 8, 16))
		var light: bool = on and cd.c.get_luminance() > 0.5
		r.add_child(UI.text(cd.name, 13, UI.BG if light else UI.TEXT, "semi"))
		p.add_child(r)
		UI.clickable(p, set_colour.bind(i))
		_keys.add_child(p)


func _refresh_stars() -> void:
	for c in _stars.get_children():
		c.queue_free()
	for i in 5:
		var k := UI.kbd(str(i + 1))
		if i < rating:
			k.add_theme_stylebox_override("panel", UI.pad(UI.style(UI.LIME, 3), 5, 1))
		UI.clickable(k, set_rating.bind(i + 1))
		_stars.add_child(k)
	_stars.add_child(UI.text("  stars, hold ⇧", 12, UI.MUTED))


func set_colour(i: int) -> void:
	_undo.append(Vector2i(colour, rating))
	colour = i
	_refresh_keys()


func set_rating(n: int) -> void:
	_undo.append(Vector2i(colour, rating))
	rating = n
	_refresh_stars()


func _next() -> void:
	if colour >= 0:
		next.emit(colour, rating)


func _tick() -> void:
	_wave.frac = pos / _length()
	_wave.queue_redraw()
	if _head.get_child_count() > 0:
		var tl := _head.get_child(_head.get_child_count() - 1) as Label
		if tl:
			tl.text = "%s / %s" % [_clock(pos), _clock(_length())]


func _clock(s: float) -> String:
	return "%d:%02d" % [int(s) / 60, int(s) % 60]


func _process(delta: float) -> void:
	if playing and not track.is_empty():
		pos = minf(pos + delta, _length())
		_tick()


func jump(n: int) -> void:
	pos = SECTIONS[clampi(n, 0, 5)] * _length()
	_tick()


func _section() -> int:
	var f := pos / _length()
	var s := 0
	for i in SECTIONS.size():
		if f >= SECTIONS[i] - 0.001:
			s = i
	return s


func handle_key(k: InputEventKey) -> bool:
	if track.is_empty():
		return false
	var key := k.physical_keycode
	if k.alt_pressed or k.ctrl_pressed or k.meta_pressed:
		return false
	for i in 8:
		if key == UI.COLOURS[i].key:
			set_colour(i)
			return true
	if key >= KEY_1 and key <= KEY_6:
		var n := key - KEY_1
		if k.shift_pressed:
			if n < 5:
				set_rating(n + 1)
		else:
			jump(n)
		return true
	match key:
		KEY_SPACE:
			playing = not playing
		KEY_LEFT:
			jump(_section() - 1)
		KEY_RIGHT:
			jump(_section() + 1)
		KEY_ENTER, KEY_KP_ENTER:
			if k.shift_pressed:
				skip.emit()
			else:
				_next()
		KEY_ESCAPE:
			playing = false
		KEY_U:
			if not _undo.is_empty():
				var u: Vector2i = _undo.pop_back()
				colour = u.x
				rating = u.y
				_refresh_keys()
				_refresh_stars()
		_:
			return false
	return true


## The whole track as bars, coloured up to the playhead, with its sections underneath.
class Wave extends Control:
	signal seek(f: float)
	var bars := PackedFloat32Array()
	var frac := 0.0
	var font: Font

	func _init(f: Font) -> void:
		font = f
		custom_minimum_size = Vector2(0, 100)
		mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND

	func build(sd: int) -> void:
		var rng := RandomNumberGenerator.new()
		rng.seed = sd
		bars.resize(132)
		for i in bars.size():
			var f := float(i) / bars.size()
			var s := 0
			for j in 6:
				if f >= [0.0, 0.17, 0.37, 0.55, 0.67, 0.88][j]:
					s = j
			bars[i] = [0.25, 0.5, 0.95, 0.35, 0.95, 0.3][s] * rng.randf_range(0.8, 1.0)
		queue_redraw()

	func _gui_input(e: InputEvent) -> void:
		var down: bool = (e is InputEventMouseButton and e.pressed and e.button_index == MOUSE_BUTTON_LEFT) \
			or (e is InputEventMouseMotion and e.button_mask & MOUSE_BUTTON_MASK_LEFT)
		if down:
			seek.emit(clampf(e.position.x / size.x, 0, 1))
			accept_event()

	func _draw() -> void:
		var h := size.y - 18
		draw_rect(Rect2(0, 0, size.x, h), Color("#222327"))
		var n := bars.size()
		if n == 0:
			return
		var w := size.x / n
		for i in n:
			var x := i * w + w * 0.5
			var a := bars[i] * h * 0.46
			var c := Color("#c8f13c") if (i + 0.5) / n <= frac else Color("#5a5b62")
			draw_line(Vector2(x, h / 2 - a), Vector2(x, h / 2 + a), c, maxf(w * 0.55, 2))
		var names := ["INTRO", "BUILD", "DROP", "BREAK", "DROP", "OUTRO"]
		var starts := [0.0, 0.17, 0.37, 0.55, 0.67, 0.88]
		for j in 6:
			var x: float = starts[j] * size.x
			if j > 0:
				draw_line(Vector2(x, 0), Vector2(x, h), Color("#1c1d20"), 2)
			draw_string(font, Vector2(x + 4, size.y - 3), "%d %s" % [j + 1, names[j]], HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color("#a0a1a8"))
		draw_line(Vector2(frac * size.x, 0), Vector2(frac * size.x, h), Color.WHITE, 1.5)
