extends VBoxContainer
## Settings, which is also the setup: vibe, library and stats unlock once music and
## rekordbox.xml are set. Keys mirror core/src/default_config.toml where they exist.

const UI := preload("res://ui/ui.gd")

var state
var app
var _body: VBoxContainer


func _init(s, a) -> void:
	state = s
	app = a
	add_theme_constant_override("separation", 0)
	var head := UI.section(UI.PANEL, 16, 0)
	head.custom_minimum_size.y = 40
	head.add_child(UI.row(8, [UI.text("Settings", 13, UI.TEXT, "semi"), UI.spacer(), UI.text(_version(), 11, UI.MUTED, "mono")]))
	add_child(head)
	_body = UI.col(10)
	_body.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	var sc := ScrollContainer.new()
	sc.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	sc.size_flags_vertical = Control.SIZE_EXPAND_FILL
	var m := UI.margin(_body, 16, 12, 16, 16)
	m.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	sc.add_child(m)
	add_child(sc)
	state.changed.connect(func() -> void:
		if is_visible_in_tree():
			refresh())
	visibility_changed.connect(func() -> void:
		if is_visible_in_tree():
			refresh())
	refresh()


func _version() -> String:
	if ClassDB.class_exists("Vibecheck"):
		return "core " + str(ClassDB.class_call_static("Vibecheck", "version"))
	return "core not loaded"


func refresh() -> void:
	for c in _body.get_children():
		c.queue_free()
	var st: Dictionary = state.settings
	_add(UI.wrap("Choose your music and rekordbox.xml first; vibe, library and stats unlock once both are set. Everything here can be changed later from the same page.", 12))
	_heading("Library")
	_add(_path("Music", "music", true))
	_add(_path("rekordbox", "xml", false))
	_add(UI.wrap("One file both ways: export the collection here from rekordbox; vibecheck writes its playlists into it."))
	var conflicts: int = state.queue.filter(func(t: Dictionary) -> bool: return t.tag == "conflict").size()
	var total: int = state.library.size()
	var tiles := UI.row(6)
	for t: Array in [["Tracks", _n(total), UI.TEXT], ["Labelled by you", _n(state.labels), UI.TEXT],
			["Conflicts", _n(conflicts), UI.AMBER]]:
		var p := UI.panel(UI.PANEL, 4, 8, 8)
		p.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		p.add_child(UI.col(2, [UI.caps(t[0]), UI.text(t[1], 18, t[2], "mono")]))
		tiles.add_child(p)
	_add(tiles)
	var left: int = total - state.embedded
	var now: String = ("now: " + state.listening) if state.listening != "" else ("done" if left == 0 else "paused")
	_add(UI.row(8, [UI.text("Analysed", 12), UI.spacer(),
		UI.text("%s / %s · %s" % [_n(state.embedded), _n(total), now], 12, UI.MUTED, "mono")]))
	var bar := UI.Bar.new(0, float(state.embedded) / maxf(total, 1), UI.LIME)
	bar.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_add(bar)
	_toggle("Analyse the rest in the background (about 5 s a track)", "listen")
	_add(UI.wrap("Only analysed tracks can be asked about and learned from. A track you label is analysed right away."))

	_heading("Calibration")
	var th := UI.text("%d labels" % st.threshold, 12, UI.TEXT, "mono")
	_add(UI.row(8, [UI.text("Start mixing in unsure tracks and conflicts after", 12), UI.spacer(), th]))
	var slider := UI.Track.new(100, 300, 10, st.threshold)
	slider.changed.connect(func(v: float) -> void: th.text = "%d labels" % int(v))
	slider.released.connect(func(v: float) -> void:
		if int(v) != st.threshold:
			state.set_setting("threshold", int(v)))
	_add(slider)
	_add(UI.row(0, [UI.text("100", 10, UI.MUTED, "mono"), UI.spacer(), UI.text("200", 10, UI.MUTED, "mono"), UI.spacer(), UI.text("300", 10, UI.MUTED, "mono")]))
	_add(UI.wrap("Below this the queue brings only calibration tracks, blind and spread across the sound of your collection. Past it, the tracks the model is least sure about, conflicts and old labels."))
	_toggle("Show the model's guess in the player after calibration", "show_guess")
	_add(_line("Re-ask old tracks blind", UI.text("rises as the library fills in", 12, UI.MUTED, "mono")))

	_heading("rekordbox")
	_toggle("Write the key into rekordbox's key field", "write_tonality", "8A")
	_toggle("Start the comment with key and energy", "write_comment", "8A - Energy 6")
	_add(UI.wrap("The rest of the comment, /* tags */ included, is kept."))
	_add(_line("Warn when the collection export is older than", UI.text("%d days" % st.warn_days, 12, UI.MUTED, "mono")))

	_heading("Cue points")
	_toggle("Write cue points", "cues_write")
	_add(UI.wrap("Only on tracks with no cues, or with generated ones (\"Cue 1\", \"Cue 2\", …). A track with any cue you set yourself is never touched."))
	var lead := UI.row(6, [UI.text("Hot cues", 12), UI.spacer()])
	for i in 2:
		lead.add_child(UI.chip(["At each change · A–D", "16 + 8 bars before · A–H"][i], st.cues_lead == i, func() -> void:
			state.set_setting("cues_lead", i)))
	_add(lead)
	var loops := UI.row(6, [UI.text("Loops", 12), UI.spacer()])
	var used := "ABCD" if st.cues_lead == 0 else "ABCDEFGH"
	for slot in "ABCDEFGH":
		loops.add_child(_slot(slot, slot in used))
	_add(loops)
	_add(UI.wrap("Click a slot to make it a loop and set its length in bars, e.g. A and G as 16-bar loops to wait at the mix-in and mix-out."))
	_toggle("Memory cue on every hot cue and the first downbeat", "cues_memory")

	var next := "Calibration first: at least %d tracks, blind." % st.threshold if state.calibrating() \
		else "%d labels so far: the queue mixes in what the model is least sure about." % state.labels
	var start := UI.row(8, [UI.wrap(next), UI.button("Start", "ENTER", true, func() -> void: app.show_screen("vibe"))])
	start.get_child(0).size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_body.add_child(UI.margin(start, 0, 10, 0, 0))


func _n(n: int) -> String:
	var s := str(n)
	var out := ""
	while s.length() > 3:
		out = "," + s.right(3) + out
		s = s.left(s.length() - 3)
	return s + out


func _add(c: Control) -> void:
	_body.add_child(c)


func _heading(t: String) -> void:
	_body.add_child(UI.margin(UI.hline(), 0, 4, 0, 4))
	_body.add_child(UI.text(t, 13, UI.TEXT, "bold"))


## A setting on one line, with a hairline under it.
func _line(t: String, right: Control) -> Control:
	var p := UI.section(Color.TRANSPARENT, 6, 0)
	(p.get_theme_stylebox("panel") as StyleBoxFlat).border_color = Color("#34353a")
	p.custom_minimum_size.y = 32
	p.add_child(UI.row(8, [UI.text(t, 12), UI.spacer(), right]))
	return p


func _toggle(t: String, key: String, example := "") -> void:
	var st: Dictionary = state.settings
	var on: bool = st[key]
	var right := UI.row(10)
	if example != "":
		right.add_child(UI.text(example, 12, UI.MUTED, "mono"))
	right.add_child(UI.chip("On" if on else "Off", on, func() -> void: state.set_setting(key, not on)))
	_add(_line(t, right))


func _path(label: String, key: String, folder: bool) -> Control:
	var p := UI.panel(UI.PANEL, 4, 10, 6)
	var s: StyleBoxFlat = p.get_theme_stylebox("panel")
	s.border_color = UI.LINE
	s.set_border_width_all(1)
	var l := UI.caps(label)
	l.custom_minimum_size.x = 72
	var v := UI.text(state.settings[key], 12, UI.TEXT, "mono")
	v.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	p.add_child(UI.row(10, [l, v, UI.button("Change", "", false, func() -> void: _pick(key, folder))]))
	return p


func _pick(key: String, folder: bool) -> void:
	var d := FileDialog.new()
	d.use_native_dialog = true
	d.access = FileDialog.ACCESS_FILESYSTEM
	d.file_mode = FileDialog.FILE_MODE_OPEN_DIR if folder else FileDialog.FILE_MODE_OPEN_FILE
	if not folder:
		d.filters = PackedStringArray(["*.xml ; rekordbox xml"])
	var done := func(path: String) -> void:
		d.queue_free()
		state.set_setting(key, path.replace(OS.get_environment("HOME"), "~"))
		app.refresh_rail()
	d.dir_selected.connect(done)
	d.file_selected.connect(done)
	d.canceled.connect(d.queue_free)
	add_child(d)
	d.popup_centered()


func _slot(slot: String, used: bool) -> Control:
	var loops: Dictionary = state.settings.cues_loops.duplicate()
	var bars: int = loops.get(slot, 0)
	var p := UI.panel(_slot_bg(bars), 3, 0, 2)
	p.custom_minimum_size = Vector2(44, 36)
	p.modulate.a = 1.0 if used else 0.35
	var t := UI.text(slot, 12, UI.TEXT, "bold")
	t.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	var sub := UI.caps("cue" if bars == 0 else "%d bars" % bars, UI.LIME if bars else UI.MUTED, 9)
	sub.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	p.add_child(UI.col(0, [t, sub]))
	UI.clickable(p, func() -> void:
		var steps := [0, 16, 8, 4]
		var nxt: int = steps[(steps.find(bars) + 1) % steps.size()]
		if nxt == 0:
			loops.erase(slot)
		else:
			loops[slot] = nxt
		state.set_setting("cues_loops", loops))
	return p


func _slot_bg(bars: int) -> Color:
	return UI.SEL if bars > 0 else UI.CARD
