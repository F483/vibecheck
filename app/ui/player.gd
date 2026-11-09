extends PanelContainer
## The player: one track, its waveform, the colour keys with beside them a tray of what you
## say about the track (stars, energy, key) level with the top row and the parts of the
## track level with the bottom one, then a centred tray of the main controls: undo | stop, play | the two buttons that keep their place and keys
## everywhere, Skip or Requeue (⇧ENTER) and Confirm (ENTER), which makes what is shown your
## label and moves on.
## Used by vibe and library.

const UI := preload("res://ui/ui.gd")

## energy 0 and key "" when you left them as they were
signal next(colour: int, rating: int, energy: int, key: String)
signal skip

const KEY_H := 44
const KEY_GAP := 6

## The skip button and the tag, per context.
const MODES := {
	"queue": {"tag": "", "skip": "Requeue", "hint": ""},
	"relisten": {"tag": "relistening · from the export list", "skip": "Skip", "hint": ""},
	"library": {"tag": "relistening · from library", "skip": "Skip", "hint": "Confirm adds it to the next Export"},
	"dropped": {"tag": "from rekordbox", "skip": "Skip", "hint": "Confirm makes it your label"},
}

var state
var track: Dictionary = {}
var mode := "queue"
## The tracks the player may load next, read ahead so their waveforms are there: set by its screen.
var ahead := Callable()
var colour := -1
var rating := 0
var energy := 0
var key := ""
var _energy0 := 0
var _key0 := ""
var playing := false
var pos := 0.0
var length := 1.0
## where each part starts, seconds; from the cue model when rekordbox has a beat grid
var sections: Array[float] = [0.0]
var _undo: Array[Array] = []
var _lit := -1
var _audio: AudioStreamPlayer

var _head: HBoxContainer
var _title: Label
var _wave: Wave
var _art: TextureRect
var _keys: GridContainer
var _stars: HBoxContainer
var _meta: HBoxContainer
var _picker: PopupPanel
var _boxes := {}
var _pick_pos := Vector2i.ZERO
var _transport: HBoxContainer
var _parts_row: HBoxContainer
var _actions: HBoxContainer
var _undo_box: HBoxContainer
## hidden with no track open, so no empty outlines show
var _trays: Array[Control] = []


func _init(bg: Color, s) -> void:
	state = s
	_audio = AudioStreamPlayer.new()
	_audio.finished.connect(func() -> void:
		playing = false
		_refresh_transport())
	add_child(_audio)
	add_theme_stylebox_override("panel", UI.pad(_bg(bg), 16, 14))
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	_head = UI.row(10)
	_head.resized.connect(_fit_title)
	_wave = Wave.new(UI.font("mono"))
	_wave.seek.connect(func(f: float) -> void: seek(f * length))
	_wave.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	# the cover in front of the waveform, as tall as the peaks
	_art = TextureRect.new()
	_art.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_art.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_COVERED
	_art.custom_minimum_size = Vector2(Wave.H - Wave.LABELS, Wave.H - Wave.LABELS)
	_art.size_flags_vertical = Control.SIZE_SHRINK_BEGIN
	_art.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_keys = GridContainer.new()
	_keys.columns = 4
	_keys.add_theme_constant_override("h_separation", KEY_GAP)
	_keys.add_theme_constant_override("v_separation", KEY_GAP)
	_stars = UI.row(4)
	_meta = UI.row(12)
	_transport = UI.row(6)
	_parts_row = UI.row(4)
	_actions = UI.row(8)
	_undo_box = UI.row(0)
	# beside the colour keys, a row each: what you say about the track (stars, energy, key)
	# in a tray level with the top keys, the parts of the track with the bottom ones
	_parts_row.custom_minimum_size.y = KEY_H
	var meta := _tray(UI.row(12, [_stars, _vline(), _meta]), false)
	meta.custom_minimum_size.y = KEY_H
	var side := UI.col(KEY_GAP, [meta, _parts_row])
	side.size_flags_vertical = Control.SIZE_SHRINK_BEGIN
	# the main controls (undo | stop, play | skip, confirm) centred in a tray under the keys
	_trays = [meta, _tray(UI.row(12, [_undo_box, _vline(), _transport, _vline(), _actions]))]
	# keys left of the centre, tray and parts right of it: two equal halves, each pushed
	# against the middle; narrower than both, they just sit side by side
	var left := UI.row(0, [_keys])
	left.alignment = BoxContainer.ALIGNMENT_END
	var right := UI.row(0, [side])
	for h: HBoxContainer in [left, right]:
		h.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	add_child(UI.col(10, [_head, UI.row(10, [_art, _wave]), UI.row(20, [left, right]), _trays[1]]))


## The title at its full width while it fits, else trimmed with …, leaving the artist line some
## room: the title never sets how narrow the window can get.
func _fit_title() -> void:
	if not is_instance_valid(_title) or _title.is_queued_for_deletion():
		return
	var others := 0.0
	var n := 0
	for c: Control in _head.get_children():
		if c.is_queued_for_deletion() or not c.visible:
			continue
		n += 1
		if c != _title and c.size_flags_horizontal != Control.SIZE_EXPAND_FILL:
			others += c.get_combined_minimum_size().x
	var f := _title.get_theme_font("font")
	var full := f.get_string_size(_title.text, HORIZONTAL_ALIGNMENT_LEFT, -1, _title.get_theme_font_size("font_size")).x
	var room := _head.size.x - others - 10 * (n - 1) - 120
	_title.custom_minimum_size.x = minf(full, maxf(room, 80))


## The main controls' tray, taken out of the player to sit elsewhere (vibe: on the export
## list's strip, centred, Export to its right). Still shown and hidden with the track.
func take_controls() -> Control:
	var wrap: Control = _trays[1]
	var t: Control = wrap.get_child(1)
	wrap.remove_child(t)
	wrap.queue_free()
	_trays[1] = t
	return t


## A centred group: dark, thin border, rounded.
func _tray(c: Control, centred := true) -> Control:
	var t := PanelContainer.new()
	t.add_theme_stylebox_override("panel", UI.pad(UI.style(UI.DARK, 8, UI.LINE, 1), 14, 8))
	t.mouse_filter = Control.MOUSE_FILTER_IGNORE
	t.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	t.add_child(c)
	return UI.row(0, [UI.spacer(), t, UI.spacer()]) if centred else t


func _vline() -> ColorRect:
	var l := ColorRect.new()
	l.color = UI.LINE
	l.custom_minimum_size = Vector2(1, 24)
	l.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	l.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return l


func _bg(c: Color) -> StyleBoxFlat:
	var s := UI.style(c)
	s.border_color = UI.LINE
	s.border_width_bottom = 1
	return s


## Load a track; blind (no colour or stars set) when `guess` is -1 on the queue.
## Keeps playing if it was: stepping through a batch is listening, not clicking play.
func show_track(t: Dictionary, m: String, guess := -1, guess_stars := -1) -> void:
	if t == track and m == mode:
		return
	track = t
	mode = m
	colour = guess if m == "queue" else int(t.colour)
	rating = maxi(guess_stars, 0) if m == "queue" else int(t.rating)
	energy = int(t.energy)
	key = str(t.key)
	_energy0 = energy
	_key0 = key
	pos = 0.0
	length = float(t.length)
	sections = [0.0]
	_undo.clear()
	_wave.clear()
	var stream := AudioStreamMP3.load_from_file(state.audio_path(t))
	_audio.stream = stream
	if stream:
		length = stream.get_length()
	if playing and stream:
		_audio.play()
	else:
		playing = false
	_refresh()
	state.wave(t, func(w: Variant) -> void:
		if w == null or track != t:
			return
		length = float(w.duration)
		_wave.show_peaks(PackedFloat32Array(w.peaks), length)
		_tick(),
	func(p: Variant) -> void:
		if p == null or track != t:
			return
		sections.clear()
		var names: Array[String] = []
		for sec: Array in p.sections:
			sections.append(float(sec[0]))
			names.append(str(sec[1]))
		_wave.show_parts(sections, names, p.cues, p.own_cues)
		# what an export would write; yours stays if you changed it meanwhile
		var we := int(p.energy)
		var wk := str(p.key)
		if energy == _energy0 and we > 0:
			energy = we
		if key == _key0 and wk != "":
			key = wk
		if we > 0:
			_energy0 = we
			t.energy = we
		if wk != "":
			_key0 = wk
			t.key = wk
		_lit = -1
		_refresh_meta()
		_tick(),
	ahead.call() if ahead.is_valid() else [])


func clear(text := "Queue is empty: sync from rekordbox or add tracks from the library") -> void:
	track = {}
	playing = false
	_audio.stop()
	_audio.stream = null
	_wave.clear()
	_art.texture = null
	for c in _head.get_children():
		c.queue_free()
	for t in _trays:
		t.visible = false
	for r: Control in [_actions, _undo_box, _transport, _parts_row, _meta, _stars]:
		for c in r.get_children():
			c.queue_free()
	_head.add_child(UI.text(text, 14, UI.MUTED))


## Play or pause where the playhead is.
func play(on: bool) -> void:
	if track.is_empty() or _audio.stream == null:
		return
	playing = on
	if on:
		_audio.play(pos)
	else:
		pos = _audio.get_playback_position() if _audio.playing else pos
		_audio.stop()
	_refresh_transport()


## Stop, and back to the start of the track.
func stop() -> void:
	play(false)
	seek(0)


## Stop and play, and the parts of the track (← one back, a button per part, → one on).
func _refresh_transport() -> void:
	for r: Control in [_transport, _parts_row]:
		for c in r.get_children():
			c.queue_free()
	if track.is_empty():
		return
	_lit = _section()
	for b: Control in _parts():
		_parts_row.add_child(b)
	_transport.add_child(_control("stop", "Stop", "ESC", stop, UI.PLAYBACK))
	var p := _control("pause" if playing else "play", "Pause" if playing else "Play", "SPACE", func() -> void: play(not playing), UI.PLAYBACK)
	p.custom_minimum_size.x = 132
	(p.get_child(0) as HBoxContainer).alignment = BoxContainer.ALIGNMENT_CENTER
	_transport.add_child(p)


func _parts() -> Array:
	var out: Array = [_square("←", false, back)]
	for i in sections.size():
		out.append(_square(str(i + 1), i == _lit, jump.bind(i)))
	out.append(_square("→", false, func() -> void: jump(_section() + 1)))
	return out


## A key-shaped button; lit for the part playing.
func _square(t: String, on: bool, cb: Callable) -> PanelContainer:
	var p := UI.panel(UI.LIME if on else Color("#d6d7db"), 3, 0, 0)
	p.custom_minimum_size = Vector2(28, 28)
	p.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	var l := UI.text(t, 12, UI.BG, "mono")
	l.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	l.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	l.size_flags_vertical = Control.SIZE_EXPAND_FILL
	p.add_child(l)
	UI.clickable(p, cb)
	return p


## Energy and key, each a value that opens a picker (N, K). Lime once changed.
func _refresh_meta() -> void:
	for c in _meta.get_children():
		c.queue_free()
	if track.is_empty():
		return
	_meta.add_child(_field("Energy", str(energy) if energy > 0 else "—", energy != _energy0, "N", pick_energy))
	_meta.add_child(_vline())
	_meta.add_child(_field("Key", key if key != "" else "—", key != _key0, "K", pick_key))


func _field(t: String, v: String, changed: bool, k: String, open: Callable) -> Control:
	var box := UI.panel(UI.CARD, 3, 8, 3)
	box.custom_minimum_size = Vector2(44, 28)
	box.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	var l := UI.text(v, 13, UI.LIME if changed else UI.TEXT, "mono")
	l.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	box.add_child(l)
	_boxes[t] = box
	# the whole field opens the picker: caption, value and the key badge alike
	var f := UI.panel(Color.TRANSPARENT, 3, 0, 0)
	f.add_child(UI.row(6, [UI.caps(t), box, UI.kbd(k)]))
	UI.clickable(f, open.bind(box))
	return f


func set_energy(e: int) -> void:
	if e == energy or e < 1 or e > 10:
		return
	_remember()
	energy = e
	_refresh_meta()


func set_key(k: String) -> void:
	if k == key:
		return
	_remember()
	key = k
	_refresh_meta()


## 1 to 10 in a row. ← → step, a digit picks (0 is 10), ENTER, N or ESC close.
func pick_energy(at: Control = null) -> void:
	var items: Array[String] = []
	for e in range(1, 11):
		items.append(str(e))
	_pick(items, 10, str(energy), func(v: String) -> void: set_energy(v.to_int()), _energy_key, at if at else _boxes.get("Energy"))


## The 24 keys as rekordbox writes them: minor (A) on top, major (B) below, round the wheel.
## ← → step round the wheel, ↑ ↓ switch minor and major, ENTER, K or ESC close.
func pick_key(at: Control = null) -> void:
	var items: Array[String] = []
	for m in ["A", "B"]:
		for n in range(1, 13):
			items.append("%d%s" % [n, m])
	_pick(items, 12, key, set_key, _key_key, at if at else _boxes.get("Key"))


## A grid of values under its field; the current one lit, a click picks and closes.
func _pick(items: Array[String], cols: int, now: String, on_pick: Callable, on_key: Callable, at: Control) -> void:
	var reopen := _picker != null and _picker.visible
	if _picker:
		_picker.queue_free()
	# stepping with the keys rebuilds the field, so keep the place it first opened at
	if not reopen and at and at.is_inside_tree():
		_pick_pos = Vector2i(at.get_screen_position()) + Vector2i(0, int(at.size.y) + 6)
	_picker = PopupPanel.new()
	_picker.add_theme_stylebox_override("panel", UI.pad(UI.style(UI.DARK, 6, Color("#55565c"), 1), 10, 10))
	var g := GridContainer.new()
	g.columns = cols
	g.add_theme_constant_override("h_separation", 4)
	g.add_theme_constant_override("v_separation", 4)
	for v in items:
		var p := UI.panel(UI.LIME if v == now else UI.CARD, 3, 0, 0)
		p.custom_minimum_size = Vector2(36, 28)
		var l := UI.text(v, 12, UI.BG if v == now else UI.TEXT, "mono")
		l.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		l.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
		l.size_flags_vertical = Control.SIZE_EXPAND_FILL
		p.add_child(l)
		UI.clickable(p, func() -> void:
			on_pick.call(v)
			_picker.hide())
		g.add_child(p)
	_picker.add_child(g)
	_picker.window_input.connect(on_key)
	add_child(_picker)
	_picker.popup(Rect2i(_pick_pos, Vector2i.ZERO))


func _energy_key(e: InputEvent) -> void:
	var ke := e as InputEventKey
	if not ke or not ke.pressed:
		return
	var c := ke.keycode
	var d := ke.physical_keycode
	var now := energy if energy > 0 else 5
	if d >= KEY_0 and d <= KEY_9:
		set_energy(10 if d == KEY_0 else d - KEY_0)
		_picker.hide()
		return
	match c:
		KEY_LEFT:
			set_energy(maxi(now - 1, 1))
		KEY_RIGHT:
			set_energy(mini(now + 1, 10))
		KEY_ENTER, KEY_KP_ENTER, KEY_N, KEY_ESCAPE:
			_picker.hide()
			return
		_:
			return
	pick_energy.call_deferred()


func _key_key(e: InputEvent) -> void:
	var ke := e as InputEventKey
	if not ke or not ke.pressed:
		return
	var n := key.to_int() if key != "" else 8
	var m := key.right(1) if key != "" else "A"
	match ke.keycode:
		KEY_LEFT:
			set_key("%d%s" % [(n + 10) % 12 + 1, m])
		KEY_RIGHT:
			set_key("%d%s" % [n % 12 + 1, m])
		KEY_UP, KEY_DOWN:
			set_key("%d%s" % [n, "B" if m == "A" else "A"])
		KEY_ENTER, KEY_KP_ENTER, KEY_K, KEY_ESCAPE:
			_picker.hide()
			return
		_:
			return
	pick_key.call_deferred()


## A player button with a drawn glyph, the same height as Skip and Save.
## Stop is rekordbox's orange, play its green, with dark ink as on Confirm.
func _control(glyph: String, t: String, key: String, cb: Callable, bg: Color) -> PanelContainer:
	var p := UI.panel(bg, 4, 10, 6)
	p.custom_minimum_size.y = 32
	p.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	p.add_child(UI.row(6, [Glyph.new(glyph, UI.BG), UI.text(t, 12, UI.BG, "bold"), UI.kbd(key, true)]))
	UI.clickable(p, cb)
	return p


func seek(s: float) -> void:
	pos = clampf(s, 0, length)
	if playing:
		_audio.play(pos)
	_tick()


func _refresh() -> void:
	for c in _head.get_children():
		c.queue_free()
	var cfg: Dictionary = MODES[mode]
	var tg: String = cfg.tag if cfg.tag != "" else str(track.tag)
	if mode == "dropped":
		tg += " · model's guess" if track.source == "model" else ""
	if tg != "":
		_head.add_child(UI.tag(tg))
	_title = UI.text(track.title, 22, UI.TEXT, "heavy")
	_title.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	_head.add_child(_title)
	# the artist line takes the room left and trims with … so the player never outgrows the window
	var by := UI.text(" · ".join(PackedStringArray([track.artist, track.mix].filter(func(x: String) -> bool: return x != ""))), 13, UI.MUTED)
	by.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	by.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_head.add_child(by)
	if cfg.hint != "":
		_head.add_child(UI.margin(UI.text(cfg.hint, 11, UI.MUTED), 0, 0, 16, 0))
	# what rekordbox measured, read only, in front of the time
	if float(track.bpm) > 0:
		var bpm := UI.row(4, [UI.text("%.0f" % float(track.bpm), 13, UI.TEXT, "mono"), UI.text("BPM", 11, UI.MUTED, "mono")])
		_head.add_child(UI.margin(bpm, 0, 0, 16, 0))
	_head.add_child(UI.text("", 12, UI.MUTED, "mono"))
	_art.texture = UI.cover_texture(track, state.cover(track, 192))
	_refresh_keys()
	_refresh_stars()
	_refresh_meta()
	for c in _actions.get_children():
		c.queue_free()
	for t in _trays:
		t.visible = true
	for c in _undo_box.get_children():
		c.queue_free()
	_undo_box.add_child(UI.button("Undo", "U", false, undo, UI.MOVE))
	var sk := UI.button(cfg.skip, "⇧ENTER", false, func() -> void: skip.emit(), UI.MOVE)
	sk.custom_minimum_size.x = 150
	(sk.get_child(0) as HBoxContainer).alignment = BoxContainer.ALIGNMENT_CENTER
	_actions.add_child(sk)
	_actions.add_child(UI.button("Confirm", "ENTER", true, _next, UI.DECIDE))
	_refresh_transport()
	_tick()
	_fit_title.call_deferred()


## The model's guess arrived after the track opened: fill it in, unless you already chose.
func fill_guess(t: Dictionary) -> void:
	if t != track or t.source != "model" or int(t.colour) < 0:
		return
	if colour < 0:
		colour = int(t.colour)
		if rating == 0:
			rating = int(t.rating)
	_refresh()


func _refresh_keys() -> void:
	for c in _keys.get_children():
		c.queue_free()
	for i in 8:
		var cd: Dictionary = UI.COLOURS[i]
		var on := i == colour
		var p := UI.panel(cd.c if on else UI.CARD, 4, 10, 0)
		p.custom_minimum_size = Vector2(124, KEY_H)
		var r := UI.row(8, [UI.kbd(UI.key_label(cd.key), on)])
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
		var k := UI.text("★" if i < rating else "☆", 20, UI.LIME if i < rating else UI.MUTED)
		UI.clickable(k, set_rating.bind(i + 1))
		_stars.add_child(k)
	_stars.add_child(UI.text(" ⇧1–5", 12, UI.MUTED))


func set_colour(i: int) -> void:
	_remember()
	colour = i
	_refresh_keys()


func set_rating(n: int) -> void:
	_remember()
	rating = n
	_refresh_stars()


func _remember() -> void:
	_undo.append([colour, rating, energy, key])


func undo() -> void:
	if _undo.is_empty():
		return
	var u: Array = _undo.pop_back()
	colour = u[0]
	rating = u[1]
	energy = u[2]
	key = u[3]
	_refresh_keys()
	_refresh_stars()
	_refresh_meta()


func _next() -> void:
	if colour >= 0:
		next.emit(colour, rating, energy if energy != _energy0 else 0, key if key != _key0 else "")


func _tick() -> void:
	_wave.frac = pos / length
	_wave.queue_redraw()
	if _head.get_child_count() > 0:
		var tl := _head.get_child(_head.get_child_count() - 1) as Label
		if tl:
			tl.text = "%s / %s" % [_clock(pos), _clock(length)]
	if not track.is_empty() and _section() != _lit:
		_refresh_transport()


func _clock(s: float) -> String:
	return "%d:%02d" % [int(s) / 60, int(s) % 60]


func _process(_delta: float) -> void:
	if playing and _audio.playing:
		pos = _audio.get_playback_position()
		_tick()


func jump(n: int) -> void:
	if n >= 0 and n < sections.size():
		seek(sections[n])


## Back to the start of this part, or the one before when just past its start.
func back() -> void:
	var i := _section()
	jump(i - 1 if pos - sections[i] < 2.0 else i)


## The part the playhead is in; just after a part starts counts as that part, so ← goes back one.
func _section() -> int:
	var s := 0
	for i in sections.size():
		if pos >= sections[i] - 0.5:
			s = i
	return s


func handle_key(k: InputEventKey) -> bool:
	if track.is_empty():
		return false
	# ⌘Z / ctrl Z undo too: the one undo that is the same on every layout
	if (k.meta_pressed or k.ctrl_pressed) and k.keycode == KEY_Z:
		undo()
		return true
	if k.alt_pressed or k.ctrl_pressed or k.meta_pressed:
		return false
	# by position: the colour grid and the number row; by the letter typed: everything else
	var key := k.physical_keycode
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
	match k.keycode:
		KEY_SPACE:
			play(not playing)
		KEY_LEFT:
			back()
		KEY_RIGHT:
			jump(_section() + 1)
		KEY_ENTER, KEY_KP_ENTER:
			if k.shift_pressed:
				skip.emit()
			else:
				_next()
		KEY_ESCAPE:
			stop()
		KEY_U:
			undo()
		KEY_N:
			pick_energy()
		KEY_K:
			pick_key()
		_:
			return false
	return true


## The whole track as bars of loudness, played part lit, with its parts underneath and its
## cue points as rekordbox draws them: lettered hot cues on top (loops shaded to their end),
## red triangles along the bottom for memory cues.
class Wave extends Control:
	signal seek(f: float)
	const FLAG := 14.0
	const H := 100.0
	const LABELS := 18.0
	var peaks := PackedFloat32Array()
	var starts: Array[float] = []
	var names: Array[String] = []
	var cues: Array = []
	var own_cues := false
	var length := 1.0
	var frac := 0.0
	var font: Font

	func _init(f: Font) -> void:
		font = f
		custom_minimum_size = Vector2(0, H)
		mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND

	func clear() -> void:
		peaks = PackedFloat32Array()
		starts = []
		names = []
		cues = []
		own_cues = false
		frac = 0.0
		queue_redraw()

	func show_peaks(p: PackedFloat32Array, len: float) -> void:
		peaks = p
		length = maxf(len, 1.0)
		queue_redraw()

	func show_parts(s: Array[float], n: Array[String], c: Array, own: bool) -> void:
		starts = s
		names = n
		cues = c
		own_cues = own
		queue_redraw()

	func _gui_input(e: InputEvent) -> void:
		var down: bool = (e is InputEventMouseButton and e.pressed and e.button_index == MOUSE_BUTTON_LEFT) \
			or (e is InputEventMouseMotion and e.button_mask & MOUSE_BUTTON_MASK_LEFT)
		if down:
			seek.emit(clampf(e.position.x / size.x, 0, 1))
			accept_event()

	func _draw() -> void:
		var h := size.y - LABELS
		draw_rect(Rect2(0, 0, size.x, h), UI.BG)
		if peaks.is_empty():
			draw_string(font, Vector2(8, h / 2 + 4), "reading the audio…", HORIZONTAL_ALIGNMENT_LEFT, -1, 11, Color("#5a5b62"))
			return
		var n := int(size.x / 3)
		var w := size.x / n
		for i in n:
			# the loudest of the peaks this bar covers
			var a0 := int(float(i) / n * peaks.size())
			var a1 := maxi(int(float(i + 1) / n * peaks.size()), a0 + 1)
			var v := 0.0
			for j in range(a0, mini(a1, peaks.size())):
				v = maxf(v, peaks[j])
			var x := i * w + w * 0.5
			var a := v * h * 0.46
			var c := Color("#c8f13c") if (i + 0.5) / n <= frac else Color("#5a5b62")
			draw_line(Vector2(x, h / 2 - a), Vector2(x, h / 2 + a), c, maxf(w * 0.6, 1.5))
		for j in starts.size():
			var x: float = starts[j] / length * size.x
			if j > 0:
				draw_line(Vector2(x, 0), Vector2(x, h), Color("#1c1d20"), 2)
			# up to the next part: parts close together keep only their number, or nothing
			var room := (starts[j + 1] / length * size.x if j + 1 < starts.size() else size.x) - x - 8
			for t: String in ["%d %s" % [j + 1, names[j].to_upper()], str(j + 1)]:
				if font.get_string_size(t, HORIZONTAL_ALIGNMENT_LEFT, -1, 10).x <= room:
					draw_string(font, Vector2(x + 4, size.y - 3), t, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color("#a0a1a8"))
					break
		_draw_cues(h)
		draw_line(Vector2(frac * size.x, 0), Vector2(frac * size.x, h), Color.WHITE, 1.5)

	func _x(seconds: float) -> float:
		return clampf(seconds / length, 0, 1) * size.x

	func _draw_cues(h: float) -> void:
		# loops first, under everything: shaded from start to end
		for c: Dictionary in cues:
			if c.end != null and int(c.num) >= 0:
				var col := Color(c.colour) if c.colour != "" else Color("#ff8c00")
				draw_rect(Rect2(_x(c.start), FLAG, _x(c.end) - _x(c.start), h - FLAG), Color(col, 0.22))
		for c: Dictionary in cues:
			var x := _x(c.start)
			if int(c.num) < 0:
				# memory cue: a red triangle on the bottom edge, clear of the hot cue flags
				draw_colored_polygon(PackedVector2Array([Vector2(x - 6, h), Vector2(x + 6, h), Vector2(x, h - 9)]), Color("#e5332a"))
				continue
			var col := Color(c.colour) if c.colour != "" else Color("#28e214")
			draw_line(Vector2(x, FLAG), Vector2(x, h), Color(col, 0.8), 1.0)
			draw_rect(Rect2(x, 0, FLAG, FLAG), col)
			var letter := "ABCDEFGH"[clampi(int(c.num), 0, 7)]
			draw_string(font, Vector2(x + 3.5, FLAG - 3), letter, HORIZONTAL_ALIGNMENT_LEFT, -1, 11, Color("#1c1d20"))
		if own_cues:
			var t := "your cues · kept on export"
			var w := font.get_string_size(t, HORIZONTAL_ALIGNMENT_LEFT, -1, 10).x
			draw_string(font, Vector2(size.x - w - 6, h - 6), t, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color("#a0a1a8"))


## Play, pause and stop, drawn so they look the same whatever the font.
class Glyph extends Control:
	var kind := ""
	var ink := Color("#e8e8ea")

	func _init(k: String, c := Color("#e8e8ea")) -> void:
		kind = k
		ink = c
		custom_minimum_size = Vector2(10, 10)
		size_flags_vertical = Control.SIZE_SHRINK_CENTER
		mouse_filter = Control.MOUSE_FILTER_IGNORE

	func _draw() -> void:
		var c := ink
		match kind:
			"play":
				draw_colored_polygon(PackedVector2Array([Vector2(1, 0), Vector2(10, 5), Vector2(1, 10)]), c)
			"pause":
				draw_rect(Rect2(1, 0, 3, 10), c)
				draw_rect(Rect2(6, 0, 3, 10), c)
			"stop":
				draw_rect(Rect2(1, 1, 8, 8), c)
