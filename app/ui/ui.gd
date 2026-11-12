extends RefCounted
## The Studio look from docs/ui/decisions.md, and the small widgets every screen shares.

const BG := Color("#1c1d20")
const PANEL := Color("#2a2b2f")
const DARK := Color("#222327")
const LINE := Color("#3a3b40")
const CARD := Color("#34353a")
const TEXT := Color("#e8e8ea")
const MUTED := Color("#a0a1a8")
const LIME := Color("#c8f13c")
const AMBER := Color("#e0b04a")
## The main controls, coloured by role (the user): moving through tracks slate (Undo,
## Skip/Requeue), playback light grey (Stop, Play), deciding green (Confirm), output aqua (Export).
const MOVE := Color("#4a5468")
const DECIDE := Color("#3ddc5a")
const PLAYBACK := Color("#d6d7db")
const OUTPUT := Color("#25e0d0")
const SEL := Color("#3d4320")
const DIM := Color(0.055, 0.055, 0.063, 0.72)

## rekordbox's eight colours: keys QWER are the light row, ASDF the dark one, by position
## (a grid of keys on any layout); everything else is matched by the letter it types.
const COLOURS := [
	{"name": "Orange", "c": Color("#ff9f1c"), "key": KEY_Q},
	{"name": "Yellow", "c": Color("#f4e04d"), "key": KEY_W},
	{"name": "Aqua", "c": Color("#25e0d0"), "key": KEY_E},
	{"name": "Pink", "c": Color("#ff2e88"), "key": KEY_R},
	{"name": "Red", "c": Color("#e5332a"), "key": KEY_A},
	{"name": "Green", "c": Color("#3ddc5a"), "key": KEY_S},
	{"name": "Blue", "c": Color("#3a66ff"), "key": KEY_D},
	{"name": "Purple", "c": Color("#8a3ad1"), "key": KEY_F},
]

## Tag colours: [background, text].
const TAGS := {
	"calibration": [Color("#3d4320"), Color("#c8f13c")],
	"conflict": [Color("#4a2c1c"), Color("#ffb07a")],
	"unsure": [Color("#1c3e3b"), Color("#6ff0e2")],
	"re-ask": [Color("#3a3b40"), Color("#d6d7db")],
	"old label": [Color("#463a1c"), Color("#e0b04a")],
	"from rekordbox": [Color("#24304a"), Color("#9fb8ff")],
	"blind": [Color("#3a3b40"), Color("#d6d7db")],
}

static var _fonts := {}


## What the key in this position prints on the keyboard in use: Q is X on Neo, A on AZERTY.
static func key_label(physical: Key) -> String:
	var l := DisplayServer.keyboard_get_label_from_physical(physical)
	return OS.get_keycode_string(l if l != KEY_NONE else physical)


## A colour key, by position: these win over letters that land on the same keys.
static func is_colour_key(k: InputEventKey) -> bool:
	for cd: Dictionary in COLOURS:
		if k.physical_keycode == cd.key:
			return true
	return false


static func font(kind := "sans") -> Font:
	if not _fonts.has(kind):
		var f := SystemFont.new()
		if kind == "mono":
			f.font_names = PackedStringArray(["IBM Plex Mono", "Menlo", "Monaco"])
			f.font_weight = 500
		else:
			f.font_names = PackedStringArray(["Inter", "Helvetica Neue", "Arial"])
			f.font_weight = {"sans": 400, "semi": 600, "bold": 700, "heavy": 800}.get(kind, 400)
		_fonts[kind] = f
	return _fonts[kind]


static func style(bg: Color, radius := 0, border := Color.TRANSPARENT, bw := 0) -> StyleBoxFlat:
	var s := StyleBoxFlat.new()
	s.bg_color = bg
	s.set_corner_radius_all(radius)
	if bw > 0:
		s.border_color = border
		s.set_border_width_all(bw)
	s.anti_aliasing = radius > 0
	return s


static func pad(s: StyleBox, h: float, v: float) -> StyleBox:
	s.content_margin_left = h
	s.content_margin_right = h
	s.content_margin_top = v
	s.content_margin_bottom = v
	return s


static func panel(bg: Color, radius := 0, h := 0.0, v := 0.0) -> PanelContainer:
	var p := PanelContainer.new()
	p.add_theme_stylebox_override("panel", pad(style(bg, radius), h, v))
	p.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return p


## A full-width band with a line under it: the stages of the vibe screen.
static func section(bg: Color, h := 16.0, v := 12.0) -> PanelContainer:
	var s := StyleBoxFlat.new()
	s.bg_color = bg
	s.border_color = LINE
	s.border_width_bottom = 1
	var p := PanelContainer.new()
	p.add_theme_stylebox_override("panel", pad(s, h, v))
	p.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return p


static func text(t: String, size := 12, color := TEXT, kind := "sans") -> Label:
	var l := Label.new()
	l.text = t
	l.add_theme_font_override("font", font(kind))
	l.add_theme_font_size_override("font_size", size)
	l.add_theme_color_override("font_color", color)
	l.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	return l


## Small uppercase mono label, the `.lbl` of the mockup.
static func caps(t: String, color := MUTED, size := 10) -> Label:
	return text(t.to_upper(), size, color, "mono")


static func wrap(t: String, size := 11, color := MUTED) -> Label:
	var l := text(t, size, color)
	l.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	l.custom_minimum_size.x = 40
	return l


static func row(sep := 8, children: Array = []) -> HBoxContainer:
	var b := HBoxContainer.new()
	b.add_theme_constant_override("separation", sep)
	b.mouse_filter = Control.MOUSE_FILTER_IGNORE
	for c: Control in children:
		b.add_child(c)
	return b


static func col(sep := 8, children: Array = []) -> VBoxContainer:
	var b := VBoxContainer.new()
	b.add_theme_constant_override("separation", sep)
	b.mouse_filter = Control.MOUSE_FILTER_IGNORE
	for c: Control in children:
		b.add_child(c)
	return b


static func margin(c: Control, l: int, t: int, r: int, b: int) -> MarginContainer:
	var m := MarginContainer.new()
	m.add_theme_constant_override("margin_left", l)
	m.add_theme_constant_override("margin_top", t)
	m.add_theme_constant_override("margin_right", r)
	m.add_theme_constant_override("margin_bottom", b)
	m.mouse_filter = Control.MOUSE_FILTER_IGNORE
	m.add_child(c)
	return m


static func spacer() -> Control:
	var c := Control.new()
	c.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	c.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return c


static func rect(color: Color, w: float, h: float) -> ColorRect:
	var r := ColorRect.new()
	r.color = color
	r.custom_minimum_size = Vector2(w, h)
	r.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	r.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return r


static func hline() -> ColorRect:
	var r := rect(LINE, 0, 1)
	r.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	return r


static func tag(t: String) -> PanelContainer:
	var cs: Array = TAGS.get(t, TAGS["re-ask"])
	if t in ["next", "early calibration", "mixing in"]:
		cs = TAGS["calibration"]
	var p := panel(cs[0], 2, 5, 2)
	p.add_child(text(t.to_upper(), 9, cs[1], "mono"))
	p.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	p.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	return p


static func kbd(t: String, dark := false) -> PanelContainer:
	var p := panel(BG if dark else Color("#d6d7db"), 3, 5, 1)
	p.add_child(text(t, 10, LIME if dark else BG, "mono"))
	p.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	return p


static func clickable(c: Control, cb: Callable) -> Control:
	c.mouse_filter = Control.MOUSE_FILTER_STOP
	c.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
	c.gui_input.connect(func(e: InputEvent) -> void:
		var m := e as InputEventMouseButton
		if m and m.pressed and m.button_index == MOUSE_BUTTON_LEFT:
			cb.call()
			c.accept_event())
	return c


## `bg` overrides the fill (lime when primary, else grey); the ink follows how light it is.
static func button(t: String, key: String, primary: bool, cb: Callable, bg := Color.TRANSPARENT) -> PanelContainer:
	var fill := bg if bg.a > 0 else (LIME if primary else LINE)
	var light := fill.get_luminance() > 0.5
	var p := panel(fill, 4, 12.0 if primary else 10.0, 6)
	p.custom_minimum_size.y = 32
	p.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	var r := row(6, [text(t, 12, BG if light else TEXT, "bold")])
	if key != "":
		r.add_child(kbd(key, light))
	p.add_child(r)
	clickable(p, cb)
	return p


static func chip(t: String, on: bool, cb: Callable = Callable()) -> PanelContainer:
	var p := panel(LIME if on else LINE, 3, 9, 4)
	p.add_child(text(t, 11, BG if on else TEXT, "bold" if on else "sans"))
	p.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	if cb.is_valid():
		clickable(p, cb)
	return p


## 14194 -> "14,194"
static func thousands(n: int) -> String:
	var s := str(absi(n))
	var out := ""
	while s.length() > 3:
		out = "," + s.right(3) + out
		s = s.left(s.length() - 3)
	return ("-" if n < 0 else "") + s + out


## The track's cover, or a gradient in its own colour when the file has none.
static func cover(t: Dictionary, side: float, art: Texture2D = null) -> TextureRect:
	var r := TextureRect.new()
	r.texture = cover_texture(t, art)
	r.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	r.custom_minimum_size = Vector2(side, side)
	r.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	r.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return r


static func cover_texture(t: Dictionary, art: Texture2D = null) -> Texture2D:
	if art:
		return art
	var g := Gradient.new()
	g.set_color(0, t.cover[0])
	g.set_color(1, t.cover[1])
	var tex := GradientTexture2D.new()
	tex.gradient = g
	tex.width = 64
	tex.height = 64
	tex.fill_to = Vector2(1, 1)
	return tex


## How far a label is trusted, 1 guessed .. 4 played: a ring filled in quarters.
class Ring extends Control:
	var level := 1

	func _init(l: int) -> void:
		level = l
		custom_minimum_size = Vector2(12, 12)
		size_flags_vertical = Control.SIZE_SHRINK_CENTER
		mouse_filter = Control.MOUSE_FILTER_IGNORE

	func _draw() -> void:
		var c := size / 2
		var col := Color("#c8f13c") if level >= 3 else (Color("#e0b04a") if level == 2 else Color("#a0a1a8"))
		draw_arc(c, 4.5, 0, TAU, 24, Color("#4a4b50"), 2.0, true)
		draw_arc(c, 4.5, -PI / 2, -PI / 2 + TAU * level / 4.0, 24, col, 2.0, true)


## A thin horizontal meter: energy, calibration progress.
class Bar extends Control:
	var frac := 0.0
	var fill := Color("#d6d7db")

	func _init(w: float, f: float, c := Color("#d6d7db")) -> void:
		frac = f
		fill = c
		custom_minimum_size = Vector2(w, 4)
		size_flags_vertical = Control.SIZE_SHRINK_CENTER
		mouse_filter = Control.MOUSE_FILTER_IGNORE

	func _draw() -> void:
		draw_rect(Rect2(Vector2.ZERO, size), Color("#3a3b40"))
		draw_rect(Rect2(Vector2.ZERO, Vector2(size.x * clampf(frac, 0, 1), size.y)), fill)


## Rail icons, drawn as lines so nothing needs importing.
class Icon extends Control:
	var kind := ""
	var color := Color("#e8e8ea")

	func _init(k: String) -> void:
		kind = k
		custom_minimum_size = Vector2(18, 18)
		size_flags_horizontal = Control.SIZE_SHRINK_CENTER
		mouse_filter = Control.MOUSE_FILTER_IGNORE

	func _draw() -> void:
		var s := size.x / 24.0
		var w := 1.5
		match kind:
			"vibe":
				var pts := PackedVector2Array()
				for i in 9:
					var a := i * PI / 4 - PI / 2
					var r := 9.5 if i % 2 == 0 else 2.6
					pts.append((Vector2(11, 13) + Vector2(cos(a), sin(a)) * r) * s)
				draw_polyline(pts, color, w, true)
				draw_line(Vector2(20, 2) * s, Vector2(20, 7) * s, color, w, true)
				draw_line(Vector2(17.5, 4.5) * s, Vector2(22.5, 4.5) * s, color, w, true)
			"library":
				for x: float in [4.0, 8.0, 12.0]:
					draw_line(Vector2(x, 4 if x == 4.0 else 6) * s, Vector2(x, 20) * s, color, w, true)
				draw_line(Vector2(16, 6) * s, Vector2(20, 20) * s, color, w, true)
			"stats":
				draw_polyline(PackedVector2Array([Vector2(3, 3) * s, Vector2(3, 21) * s, Vector2(21, 21) * s]), color, w, true)
				draw_line(Vector2(8, 17) * s, Vector2(8, 13) * s, color, w, true)
				draw_line(Vector2(13, 17) * s, Vector2(13, 7) * s, color, w, true)
				draw_line(Vector2(18, 17) * s, Vector2(18, 10) * s, color, w, true)
			"history":
				# a clock with an arrow running back round it
				var c := Vector2(12, 12) * s
				draw_arc(c, 8.5 * s, -PI * 0.75, PI, 28, color, w, true)
				draw_polyline(PackedVector2Array([Vector2(2.5, 3.5) * s, Vector2(6, 6) * s, Vector2(2.5, 8.5) * s]), color, w, true)
				draw_polyline(PackedVector2Array([Vector2(12, 7) * s, c, Vector2(15.5, 14) * s]), color, w, true)
			"settings":
				var c := Vector2(12, 12) * s
				draw_arc(c, 3 * s, 0, TAU, 20, color, w, true)
				draw_arc(c, 7 * s, 0, TAU, 28, color, w, true)
				for i in 8:
					var d := Vector2(cos(i * PI / 4), sin(i * PI / 4))
					draw_line(c + d * 7 * s, c + d * 10 * s, color, w + 0.6, true)


## A slider drawn like the mockup: a thin track and a lime knob.
class Track extends Control:
	signal changed(v: float)
	signal released(v: float)
	var lo := 0.0
	var hi := 1.0
	var step := 1.0
	var value := 0.0

	func _init(l: float, h: float, st: float, v: float) -> void:
		lo = l
		hi = h
		step = st
		value = v
		custom_minimum_size = Vector2(0, 16)
		mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND

	func _gui_input(e: InputEvent) -> void:
		if e is InputEventMouseButton and not e.pressed and e.button_index == MOUSE_BUTTON_LEFT:
			released.emit(value)
		var down: bool = (e is InputEventMouseButton and e.pressed and e.button_index == MOUSE_BUTTON_LEFT) \
			or (e is InputEventMouseMotion and e.button_mask & MOUSE_BUTTON_MASK_LEFT)
		if down:
			var f := clampf((e.position.x - 8) / maxf(size.x - 16, 1), 0, 1)
			value = snappedf(lo + f * (hi - lo), step)
			changed.emit(value)
			queue_redraw()
			accept_event()

	func _draw() -> void:
		draw_rect(Rect2(0, 6, size.x, 4), Color("#3a3b40"))
		var x := 8 + (value - lo) / (hi - lo) * (size.x - 16)
		draw_circle(Vector2(x, 8), 8, Color("#c8f13c"), true, -1, true)
