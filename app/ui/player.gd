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

## Why a track is in front of you, as its tag says: [tag, why, what to do]. Click the tag.
const WHY := [
	["calibration", "few labels yet: tracks spread across your collection's sound, asked blind",
		"Pick its colour and stars. These show the model your whole range."],
	["unsure", "not labelled by you; the model is least sure of its colour",
		"Confirm the guess if it's right, or pick the right colour. These teach the model most."],
	["conflict", "your label, but the model is sure of another colour; asked blind",
		"Pick your colour again to keep it, or another to correct it. Either way it counts as confirmed."],
	["old label", "a colour from before vibecheck the model disagrees with; asked blind",
		"Pick the old colour to keep it, or another to correct it. Either becomes your label."],
	["re-ask", "you put it in the queue from the library; asked blind",
		"Pick its colour again. It replaces your label."],
	["blind", "a check card: asked blind, so your answer stays your own and the model's accuracy stays honest",
		"Pick its colour and stars as if there were no guess."],
	["cue conflict", "your cues, but the cue model, having learned them, would put them elsewhere; its cues show faint",
		"Keep yours with Confirm, move them, or take the model's. Either way they count as confirmed."],
	["from rekordbox", "you dropped it on the app",
		"Confirm makes it your label."],
	["relistening", "opened from the export list or the library",
		"Change what's wrong. Confirm puts it in the next Export."],
]

var state
var track: Dictionary = {}
var mode := "queue"
## The model's guess the track opened with, UI colour index and stars, -1 when none was
## shown: saved with the label, so a kept guess is trusted less than a blind answer.
var shown := -1
var shown_stars := -1
## The tracks the player may load next, read ahead so their waveforms are there: set by its screen.
var ahead := Callable()
var colour := -1
var rating := 0
var energy := 0
var key := ""
var _energy0 := 0
var _why: PopupPanel
## The screen the player is docked in: in the library, Skip goes to the next row whatever
## the track, so it never says Requeue.
var where := "vibe"
var _key0 := ""
var playing := false
var pos := 0.0
var length := 1.0
## where each part starts, seconds; from the cue model when rekordbox has a beat grid
var sections: Array[float] = [0.0]
var _undo: Array[Array] = []
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
var _actions: HBoxContainer
var _undo_box: HBoxContainer
## hidden with no track open, so no empty outlines show
var _trays: Array[Control] = []
## Cue mode (Tab): the eight colour keys are the hot cues A–H, by the same key positions, in
## two rows of four as a DDJ controller's pads; ⇧ + pad deletes, as there.
## Your cues as edited ({num, start, end}: num 0..7 hot, -1 memory; seconds), the model's
## suggestion they started from, the one selected, and rekordbox's grid to place them on.
var cue_mode := false
var cues_now: Array = []
var cues_shown: Array = []
var cue_sel := -1
var _cues_ready := false
var _beats := PackedFloat64Array()
var _bars := PackedFloat64Array()
var _p_cues: Array = []
var _p_own := false
var _switch: HBoxContainer
var _switch_room: Control
var _tools_row: HBoxContainer
## Tags: rekordbox's My Tags for the track, read from its comment; shown in place of the keys
## (a placeholder: editing them comes later)
var tags_view := false
var _tags: Array = []
var _tags_ready := false
var _tags_box: VBoxContainer
var _tags_note: Control
## loop lengths in beats, as rekordbox counts them
const LOOPS := [0, 4, 8, 16, 32, 64]


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
	_wave.pick.connect(func(i: int) -> void:
		_remember()
		cue_sel = i
		_cues_changed())
	_wave.drag.connect(_move_cue)
	_wave.drop.connect(func(i: int) -> void:
		cue_sel = i
		delete_cue())
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
	_actions = UI.row(8)
	_undo_box = UI.row(0)
	# beside the colour keys, level with the top ones: what you say about the track (stars,
	# energy, key) in a tray, or in Cues the cue keys
	var meta := _tray(UI.row(12, [_stars, _vline(), _meta]), false)
	meta.custom_minimum_size.y = KEY_H
	_switch = UI.row(6)
	_switch.custom_minimum_size.y = 24
	_switch_room = Control.new()
	_switch_room.custom_minimum_size.y = 24
	# in cue mode the cue keys take the tray's place: stars, energy and key belong to Colour
	_tools_row = UI.row(4)
	_tools_row.custom_minimum_size.y = KEY_H
	_tools_row.visible = false
	_tags_note = UI.wrap("rekordbox's My Tags, read from the track's comment, where rekordbox can write them. Editing them here comes later.", 12)
	_tags_note.custom_minimum_size = Vector2(420, KEY_H)
	_tags_note.visible = false
	var side := UI.col(KEY_GAP, [_switch_room, meta, _tools_row, _tags_note])
	side.size_flags_vertical = Control.SIZE_SHRINK_BEGIN
	# the main controls (undo | stop, play | skip, confirm) centred in a tray under the keys
	# no buttons for the parts: the hot cue pads jump through the track (the user); 1–6 and ← →
	# still do from the keyboard
	_trays = [meta, _tray(UI.row(12, [_undo_box, _vline(), _transport, _vline(), _actions]))]
	# keys left of the centre, tray and parts right of it: two equal halves, each pushed
	# against the middle; narrower than both, they just sit side by side
	_tags_box = UI.col(8)
	_tags_box.custom_minimum_size = Vector2(4 * 124 + 3 * KEY_GAP, 2 * KEY_H + KEY_GAP)
	_tags_box.visible = false
	var left := UI.row(0, [UI.col(KEY_GAP, [_switch, _keys, _tags_box])])
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


## Docked in a screen (vibe or library): the main controls go into `controls`. The player
## itself moves to the screen shown (vibe.lend), and keeps playing as it moves.
func dock(controls: Control, w: String) -> void:
	if w != where:
		where = w
		if not track.is_empty():
			_refresh()
	if _trays[1].get_parent() != controls:
		_trays[1].reparent(controls, false)


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
	# relistening to a track the model labelled shows its guess too
	var model_label: bool = m != "queue" and t.get("source", "") == "model" and int(t.colour) >= 0
	shown = guess if m == "queue" else (int(t.colour) if model_label else -1)
	shown_stars = guess_stars if m == "queue" else (int(t.rating) if model_label else -1)
	energy = int(t.energy)
	key = str(t.key)
	_energy0 = energy
	_key0 = key
	pos = 0.0
	length = float(t.length)
	sections = [0.0]
	_undo.clear()
	_wave.clear()
	cues_now = []
	cues_shown = []
	cue_sel = -1
	_cues_ready = false
	_p_cues = []
	_p_own = false
	_tags = []
	_tags_ready = false
	var stream := AudioStreamMP3.load_from_file(state.audio_path(t))
	if stream:
		# at the end, back to the start and on playing (the user)
		stream.loop = true
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
		# the cues to edit: yours if set here before, else the model's (a fresh start)
		_p_cues = p.cues
		_p_own = p.own_cues
		_beats = PackedFloat64Array(p.beats)
		_bars = PackedFloat64Array(p.bars)
		cues_shown = _plain(p.suggested)
		# a cue conflict asks about yours: they start on the pads, the model's show faint
		var conflict: bool = mode == "queue" and str(track.get("tag", "")) == "cue conflict"
		cues_now = _plain(p.cues) if p.set or conflict else cues_shown.duplicate(true)
		_wave.ghosts = cues_shown if conflict else []
		_cues_ready = not _beats.is_empty()
		_tags = p.tags
		_tags_ready = true
		_cues_changed()
		# start where the track does, past the intro, unless you already moved; 1 goes back
		if sections.size() > 1 and pos < 2.0:
			seek(sections[1])
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
	_tools_row.visible = false
	for r: Control in [_actions, _undo_box, _transport, _meta, _stars, _tools_row]:
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


## Stop and play.
func _refresh_transport() -> void:
	for c in _transport.get_children():
		c.queue_free()
	if track.is_empty():
		return
	_transport.add_child(_control("stop", "Stop", "ESC", stop, UI.PLAYBACK))
	var p := _control("pause" if playing else "play", "Pause" if playing else "Play", "SPACE", func() -> void: play(not playing), UI.PLAYBACK)
	p.custom_minimum_size.x = 132
	(p.get_child(0) as HBoxContainer).alignment = BoxContainer.ALIGNMENT_CENTER
	_transport.add_child(p)


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


## The tag's popup, for screenshots (--dialog=why).
func open_why() -> void:
	for c in _head.get_children():
		if c is PanelContainer:
			c.gui_input.emit(_click())
			return


static func _click() -> InputEventMouseButton:
	var e := InputEventMouseButton.new()
	e.pressed = true
	e.button_index = MOUSE_BUTTON_LEFT
	return e


## What each tag means, this track's lit, and what to do with it: like the trust popup.
func _why_popup(chip: Control, tg: String) -> void:
	if _why:
		_why.queue_free()
	_why = PopupPanel.new()
	_why.add_theme_stylebox_override("panel", UI.pad(UI.style(UI.DARK, 6, Color("#55565c"), 1), 12, 10))
	var v := UI.col(6, [UI.caps("Why this track is asked")])
	var g := GridContainer.new()
	g.columns = 2
	g.add_theme_constant_override("h_separation", 10)
	g.add_theme_constant_override("v_separation", 6)
	var todo := ""
	for row: Array in WHY:
		var on: bool = tg.begins_with(row[0])
		if on:
			todo = row[2]
		g.add_child(UI.tag(row[0]))
		g.add_child(UI.text(row[1], 12, UI.TEXT if on else UI.MUTED, "bold" if on else "sans"))
	v.add_child(g)
	if todo != "":
		v.add_child(UI.margin(UI.text(todo, 12, UI.LIME), 0, 4, 0, 0))
	if mode == "queue":
		v.add_child(UI.text("Requeue asks it again later.", 11, UI.MUTED))
	_why.add_child(v)
	add_child(_why)
	_why.popup(Rect2i(Vector2i(chip.get_screen_position() + Vector2(0, chip.size.y + 6)), Vector2i.ZERO))


func _refresh() -> void:
	for c in _head.get_children():
		c.queue_free()
	var cfg: Dictionary = MODES[mode]
	var tg: String = cfg.tag if cfg.tag != "" else str(track.tag)
	if mode == "dropped":
		tg += " · model's guess" if track.source == "model" else ""
	if tg != "":
		var chip := UI.tag(tg)
		UI.clickable(chip, _why_popup.bind(chip, tg))
		_head.add_child(chip)
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
	_refresh_tools()
	for c in _undo_box.get_children():
		c.queue_free()
	_undo_box.add_child(UI.button("Undo", "U", false, undo, UI.MOVE))
	var sk := UI.button("Skip" if where == "library" else cfg.skip, "⇧ENTER", false, func() -> void: skip.emit(), UI.MOVE)
	sk.custom_minimum_size.x = 150
	(sk.get_child(0) as HBoxContainer).alignment = BoxContainer.ALIGNMENT_CENTER
	_actions.add_child(sk)
	_actions.add_child(UI.button("Confirm", "ENTER", true, _next, UI.DECIDE))
	_refresh_transport()
	_tick()
	_fit_title.call_deferred()


## The model's guess arrived after the track opened: fill it in, unless you already chose.
## Which card is asking: the queue's tag, else how the track was opened.
func card() -> String:
	return str(track.get("tag", "")) if mode == "queue" else mode


func fill_guess(t: Dictionary) -> void:
	if t != track or t.source != "model" or int(t.colour) < 0:
		return
	if colour < 0:
		colour = int(t.colour)
		shown = colour
		if rating == 0:
			rating = int(t.rating)
			shown_stars = rating
	_refresh()


func _refresh_keys() -> void:
	for c in _keys.get_children():
		c.queue_free()
	_refresh_switch()
	_keys.visible = not tags_view
	_tags_box.visible = tags_view
	if tags_view:
		_refresh_tags()
		return
	if cue_mode:
		for slot in 8:
			_keys.add_child(_pad(slot))
		return
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
	_undo.append([colour, rating, energy, key, cues_now.duplicate(true), cue_sel])


func undo() -> void:
	if _undo.is_empty():
		return
	var u: Array = _undo.pop_back()
	colour = u[0]
	rating = u[1]
	energy = u[2]
	key = u[3]
	cues_now = u[4]
	cue_sel = u[5]
	_show_cues()
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
	if k.keycode == KEY_TAB:
		var vs := views()
		set_view(vs[(vs.find(view()) + (-1 if k.shift_pressed else 1) + vs.size()) % vs.size()])
		return true
	# by position: the colour grid and the number row; by the letter typed: everything else
	var key := k.physical_keycode
	for i in 8:
		if key == UI.COLOURS[i].key:
			if tags_view:
				return true  # nothing on these keys here yet
			if cue_mode:
				pad(i, k.shift_pressed)
			else:
				set_colour(i)
			return true
	if cue_mode and _cue_key(k):
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


# --- cues ------------------------------------------------------------------------------

## Cues are edited in the player when Settings › Cue points asks for it and the track has a grid.
func cues_on() -> bool:
	return bool(state.settings.get("cues_edit", true)) and _cues_ready


## Your cues and the suggestion they started from, for Confirm; empty when the player doesn't
## ask for cues (then nothing is saved and Export does as before).
func cue_decision() -> Array:
	return [cues_now, cues_shown] if cues_on() else []


## The views the switch offers, in its order: Cues (when the player asks for cues), Label, Tags.
func views() -> Array[String]:
	var out: Array[String] = ["label", "tags"]
	if bool(state.settings.get("cues_edit", true)):
		out.push_front("cues")
	return out


func view() -> String:
	return "tags" if tags_view else "cues" if cue_mode else "label"


func set_view(v: String) -> void:
	if v not in views():
		return
	tags_view = v == "tags"
	set_cue_mode(v == "cues")


func set_cue_mode(on: bool) -> void:
	if on and not bool(state.settings.get("cues_edit", true)):
		return
	if on:
		tags_view = false
	cue_mode = on
	if not on:
		cue_sel = -1
	_refresh_keys()
	_refresh_tools()
	_show_cues()


func _plain(src: Array) -> Array:
	var out: Array = []
	for c: Dictionary in src:
		out.append({"num": int(c.num), "start": float(c.start), "end": null if c.end == null else float(c.end)})
	return out


## The two-way switch above the keys: Cues | Label (colour, stars, energy, key), Tab.
func _refresh_switch() -> void:
	for c in _switch.get_children():
		c.queue_free()
	_switch.alignment = BoxContainer.ALIGNMENT_END
	if cue_mode:
		# rekordbox's Quantize: cues land on the grid's beats; off, where the playhead is
		var q := quantize()
		var qp := UI.panel(UI.LIME if q else UI.CARD, 3, 10, 2)
		qp.add_child(UI.text("Quantize", 11, UI.BG if q else UI.MUTED, "semi"))
		UI.clickable(qp, func() -> void:
			state.set_setting("cues_quantize", not q)
			_refresh_keys())
		_switch.add_child(qp)
		_switch.add_child(UI.margin(Control.new(), 0, 0, 6, 0))
	for v in views():
		var lit := v == view()
		var p := UI.panel(UI.LIME if lit else UI.CARD, 3, 10, 2)
		p.add_child(UI.text(v.capitalize(), 11, UI.BG if lit else UI.MUTED, "semi"))
		UI.clickable(p, set_view.bind(v))
		_switch.add_child(p)
	# the key badge does what its key does, as N and K do
	var tab := UI.kbd("TAB")
	UI.clickable(tab, func() -> void:
		var vs := views()
		set_view(vs[(vs.find(view()) + 1) % vs.size()]))
	_switch.add_child(tab)


## The track's tags as chips, in place of the keys.
func _refresh_tags() -> void:
	for c in _tags_box.get_children():
		c.queue_free()
	_tags_box.add_child(UI.caps("Tags"))
	if not _tags_ready:
		_tags_box.add_child(UI.text("reading…", 12, UI.MUTED))
		return
	if _tags.is_empty():
		_tags_box.add_child(UI.text("No tags on this track.", 12, UI.MUTED))
		return
	var flow := HFlowContainer.new()
	flow.add_theme_constant_override("h_separation", 6)
	flow.add_theme_constant_override("v_separation", 6)
	for t: String in _tags:
		var p := UI.panel(UI.CARD, 3, 10, 5)
		p.add_child(UI.text(t, 13, UI.TEXT, "semi"))
		flow.add_child(p)
	_tags_box.add_child(flow)


func quantize() -> bool:
	return bool(state.settings.get("cues_quantize", true))


func _hot(slot: int) -> int:
	for i in cues_now.size():
		if int(cues_now[i].num) == slot:
			return i
	return -1


## Changed from the suggestion: lime, as a corrected energy or key.
func _edited(c: Dictionary) -> bool:
	for s: Dictionary in cues_shown:
		if int(s.num) == int(c.num) and absf(float(s.start) - float(c.start)) < 0.01 and (s.end == null) == (c.end == null) \
				and (c.end == null or absf(float(s.end) - float(c.end)) < 0.01):
			return false
	return true


## One pad: its key, the slot in its colour (green cue, orange loop), where it sits as bar.beat.
func _pad(slot: int) -> PanelContainer:
	var i := _hot(slot)
	var p := UI.panel(Color("#3a3b40") if i >= 0 and i == cue_sel else UI.CARD, 4, 10, 0)
	if i >= 0 and i == cue_sel:
		var sb: StyleBoxFlat = p.get_theme_stylebox("panel")
		sb.border_color = Color.WHITE
		sb.set_border_width_all(2)
	p.custom_minimum_size = Vector2(124, KEY_H)
	var c: Dictionary = cues_now[i] if i >= 0 else {}
	var loop := i >= 0 and c.end != null
	var sq := UI.panel(Color("#ff8c00") if loop else Color("#28e214") if i >= 0 else Color("#4a4b52"), 2, 0, 0)
	sq.custom_minimum_size = Vector2(16, 16)
	sq.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	var l := UI.text("ABCDEFGH"[slot], 11, UI.BG, "mono")
	l.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	sq.add_child(l)
	var where := UI.text(_bar_beat(float(c.start)) if i >= 0 else "—", 12, UI.LIME if i >= 0 and _edited(c) else UI.TEXT if i >= 0 else UI.MUTED, "mono")
	var what := UI.text(("loop %d" % _loop_beats(i)) if loop else "cue" if i >= 0 else "empty", 9, Color("#ffb07a") if loop else UI.MUTED, "mono")
	var col := UI.col(0, [where, what])
	col.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	p.add_child(UI.row(7, [UI.kbd(UI.key_label(UI.COLOURS[slot].key)), sq, col]))
	UI.clickable(p, pad.bind(slot, false))
	return p


## Colour: the tray with stars, energy and key. Cues: the cue keys in its place.
func _refresh_tools() -> void:
	for c in _tools_row.get_children():
		c.queue_free()
	_trays[0].visible = not cue_mode and not tags_view and not track.is_empty()
	_tools_row.visible = cue_mode and not track.is_empty()
	_tags_note.visible = tags_view and not track.is_empty()
	if cue_mode:
		for b: Control in _cue_tools():
			_tools_row.add_child(b)


## The cue keys, clickable.
func _cue_tools() -> Array:
	var out: Array = []
	for t: Array in [["← →", "beat", nudge.bind(1)], ["⇧← →", "bar", nudge.bind(4)], ["O", "loop", cycle_loop],
			["M", "memory", add_memory], ["⇧ pad", "delete", delete_cue]]:
		var p := UI.panel(UI.CARD, 3, 8, 0)
		p.custom_minimum_size.y = 30
		p.size_flags_vertical = Control.SIZE_SHRINK_CENTER
		p.add_child(UI.row(6, [UI.kbd(t[0]), UI.text(t[1], 12, UI.TEXT, "semi")]))
		UI.clickable(p, t[2])
		out.append(p)
	return out


## A pad as on a DDJ controller: an empty one sets its cue at the playhead; a set one jumps
## there and plays; ⇧ + pad deletes it (to move one: delete and set again, or ← →).
func pad(slot: int, shift: bool) -> void:
	if not _cues_ready:
		return
	var i := _hot(slot)
	if shift:
		if i >= 0:
			cue_sel = i
			delete_cue()
		return
	if i < 0:
		_remember()
		cues_now.append({"num": slot, "start": _snap(pos), "end": null})
		i = cues_now.size() - 1
	else:
		seek(float(cues_now[i].start))
		if not playing:
			play(true)
	cue_sel = i
	_cues_changed()


## ← → a beat, ⇧ a bar; O loop; M memory; ⌫ delete.
func _cue_key(k: InputEventKey) -> bool:
	match k.keycode:
		KEY_LEFT, KEY_RIGHT:
			if cue_sel < 0:
				return false
			nudge((4 if k.shift_pressed else 1) * (-1 if k.keycode == KEY_LEFT else 1))
		KEY_O:
			cycle_loop()
		KEY_M:
			add_memory()
		KEY_BACKSPACE, KEY_DELETE:
			delete_cue()
		_:
			return false
	return true


func nudge(beats: int) -> void:
	if cue_sel < 0:
		return
	_remember()
	_place(cue_sel, _beat_i(float(cues_now[cue_sel].start)) + beats)
	_cues_changed()


## Off, 4, 8, 16, 32, 64 beats, round again.
func cycle_loop() -> void:
	if cue_sel < 0:
		return
	_remember()
	var c: Dictionary = cues_now[cue_sel]
	var lb := _loop_beats(cue_sel)
	var n: int = LOOPS[(LOOPS.find(lb) + 1) % LOOPS.size()] if lb in LOOPS else 4
	c.end = null if n == 0 else _loop_end(float(c.start), n)
	_cues_changed()


func add_memory() -> void:
	if not _cues_ready:
		return
	var at := _snap(pos)
	var mem := 0
	for i in cues_now.size():
		if int(cues_now[i].num) < 0:
			mem += 1
			if absf(float(cues_now[i].start) - at) < 0.01:
				cue_sel = i
				_cues_changed()
				return
	if mem >= 10:
		return
	_remember()
	cues_now.append({"num": -1, "start": at, "end": null})
	cue_sel = cues_now.size() - 1
	_cues_changed()


func delete_cue() -> void:
	if cue_sel < 0 or cue_sel >= cues_now.size():
		return
	_remember()
	cues_now.remove_at(cue_sel)
	cue_sel = -1
	_cues_changed()


## Dragged on the waveform: to the nearest beat (to the spot, with Quantize off), its loop
## keeping its length.
func _move_cue(i: int, seconds: float) -> void:
	if i < 0 or i >= cues_now.size():
		return
	if quantize():
		_place(i, _beat_i(seconds))
	else:
		var c: Dictionary = cues_now[i]
		var lb := _loop_beats(i)
		c.start = clampf(seconds, 0.0, length)
		if lb > 0:
			c.end = _loop_end(c.start, lb)
	cue_sel = i
	_cues_changed()


## A cue to beat `b`, a loop keeping its length in beats.
func _place(i: int, b: int) -> void:
	var c: Dictionary = cues_now[i]
	var lb := _loop_beats(i)
	b = clampi(b, 0, _beats.size() - 1)
	c.start = _beats[b]
	if lb > 0:
		c.end = _loop_end(c.start, lb)


## `beats` beats on from `start`, by the grid's tempo there.
func _loop_end(start: float, beats: int) -> float:
	var b := _beat_i(start)
	return start + _beat_at(b + beats) - _beat_at(b)


func _loop_beats(i: int) -> int:
	if i < 0 or cues_now[i].end == null:
		return 0
	return _beat_i(float(cues_now[i].end)) - _beat_i(float(cues_now[i].start))


func _beat_i(t: float) -> int:
	if _beats.is_empty():
		return 0
	var i := _beats.bsearch(t)
	if i >= _beats.size():
		# past the grid: extrapolate, so a loop running off the end keeps its length
		var step := _beats[-1] - _beats[-2] if _beats.size() > 1 else 0.5
		return _beats.size() - 1 + roundi((t - _beats[-1]) / step)
	if i > 0 and t - _beats[i - 1] < _beats[i] - t:
		return i - 1
	return i


func _beat_at(i: int) -> float:
	if i < _beats.size():
		return _beats[maxi(i, 0)]
	var step := _beats[-1] - _beats[-2] if _beats.size() > 1 else 0.5
	return _beats[-1] + (i - _beats.size() + 1) * step


func _snap(t: float) -> float:
	if _beats.is_empty() or not quantize():
		return t
	return _beats[clampi(_beat_i(t), 0, _beats.size() - 1)]


## "33.1": the bar from rekordbox's grid, counted from 1, and the beat in it; "~33.1" for a
## cue off the grid (set with Quantize off), at the beat nearest.
func _bar_beat(t: float) -> String:
	var off := "~" if absf(_beat_at(_beat_i(t)) - t) > 0.02 else ""
	var bar := _bars.bsearch(t + 0.01)
	if bar == 0:
		return "%s0.%d" % [off, _beat_i(t) + 1]
	var beat := _beat_i(t) - _beat_i(_bars[bar - 1])
	return "%s%d.%d" % [off, bar, beat + 1]


func _cues_changed() -> void:
	_show_cues()
	_refresh_keys()


## What the waveform shows: your cues as edited when the player asks for them, else what an
## export would write.
func _show_cues() -> void:
	if not cues_on():
		_wave.edit = false
		_wave.sel = -1
		_wave.cues = _p_cues
		_wave.own_cues = _p_own
		_wave.queue_redraw()
		return
	var shown_list: Array = []
	for c: Dictionary in cues_now:
		var col := "" if int(c.num) < 0 else "#ff8c00" if c.end != null else "#28e214"
		shown_list.append({"num": c.num, "start": c.start, "end": c.end, "colour": col})
	_wave.cues = shown_list
	_wave.own_cues = false
	_wave.sel = cue_sel
	_wave.edit = cue_mode
	_wave.queue_redraw()


## The whole track as bars of loudness, played part lit, with its parts underneath and its
## cue points as rekordbox draws them: lettered hot cues on top (loops shaded to their end),
## red triangles along the bottom for memory cues.
class Wave extends Control:
	signal seek(f: float)
	## cue mode: a cue clicked (selected), dragged to a time, right-clicked (deleted)
	signal pick(i: int)
	signal drag(i: int, seconds: float)
	signal drop(i: int)
	const FLAG := 14.0
	const H := 100.0
	const LABELS := 18.0
	var peaks := PackedFloat32Array()
	var starts: Array[float] = []
	var names: Array[String] = []
	var cues: Array = []
	## the model's cues, faint, to compare yours with (a cue conflict)
	var ghosts: Array = []
	var own_cues := false
	var sel := -1
	var edit := false
	var _dragging := -1
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
		ghosts = []
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

	## The cue under the mouse: a hot cue by its flag or line, a memory cue by its triangle.
	func _cue_at(p: Vector2) -> int:
		var h := size.y - LABELS
		for i in cues.size():
			var x := _x(float(cues[i].start))
			if int(cues[i].num) >= 0:
				if p.y < h - 12 and p.x >= x - 4 and p.x <= x + FLAG + 2:
					return i
			elif p.y >= h - 14 and p.y <= h + 2 and absf(p.x - x) <= 8:
				return i
		return -1

	func _gui_input(e: InputEvent) -> void:
		if edit:
			var b := e as InputEventMouseButton
			if b and b.pressed and (b.button_index == MOUSE_BUTTON_LEFT or b.button_index == MOUSE_BUTTON_RIGHT):
				var i := _cue_at(b.position)
				if i >= 0:
					if b.button_index == MOUSE_BUTTON_RIGHT:
						drop.emit(i)
					else:
						_dragging = i
						pick.emit(i)
					accept_event()
					return
			if b and not b.pressed and b.button_index == MOUSE_BUTTON_LEFT and _dragging >= 0:
				_dragging = -1
				accept_event()
				return
			var m := e as InputEventMouseMotion
			if m and _dragging >= 0:
				if m.button_mask & MOUSE_BUTTON_MASK_LEFT:
					drag.emit(_dragging, clampf(m.position.x / size.x, 0, 1) * length)
				else:
					_dragging = -1
				accept_event()
				return
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
		# the model's, faint and outlined, under yours
		var faint := Color(1, 1, 1, 0.35)
		for g: Dictionary in ghosts:
			var gx := _x(g.start)
			if int(g.num) < 0:
				draw_polyline(PackedVector2Array([Vector2(gx - 6, h), Vector2(gx + 6, h), Vector2(gx, h - 9), Vector2(gx - 6, h)]), faint, 1.0)
				continue
			draw_dashed_line(Vector2(gx, FLAG), Vector2(gx, h), faint, 1.0, 3.0)
			draw_rect(Rect2(gx, 0, FLAG, FLAG), faint, false, 1.0)
			draw_string(font, Vector2(gx + 3.5, FLAG - 3), "ABCDEFGH"[clampi(int(g.num), 0, 7)], HORIZONTAL_ALIGNMENT_LEFT, -1, 11, faint)
		# loops first, under everything: shaded from start to end
		for c: Dictionary in cues:
			if c.end != null and int(c.num) >= 0:
				var col := Color(c.colour) if c.colour != "" else Color("#ff8c00")
				draw_rect(Rect2(_x(c.start), FLAG, _x(c.end) - _x(c.start), h - FLAG), Color(col, 0.22))
		for ci in cues.size():
			var c: Dictionary = cues[ci]
			var x := _x(c.start)
			if int(c.num) < 0:
				# memory cue: a red triangle on the bottom edge, clear of the hot cue flags
				var tri := PackedVector2Array([Vector2(x - 6, h), Vector2(x + 6, h), Vector2(x, h - 9)])
				draw_colored_polygon(tri, Color("#e5332a"))
				if ci == sel:
					draw_polyline(PackedVector2Array([tri[0], tri[1], tri[2], tri[0]]), Color.WHITE, 1.5)
				continue
			if ci == sel:
				draw_line(Vector2(x, FLAG), Vector2(x, h), Color.WHITE, 1.5)
				draw_rect(Rect2(x - 1.5, -1.5, FLAG + 3, FLAG + 3), Color.WHITE)
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
