extends VBoxContainer
## Settings, which is also the setup: vibe, library and stats unlock once music and
## rekordbox.xml are set. Keys mirror core/src/default_config.toml where they exist.

const UI := preload("res://ui/ui.gd")

var state
var app
var _body: VBoxContainer
## Where _add puts things: the body, or the card being filled.
var _col: VBoxContainer
## The column _card puts its card into.
var _side: VBoxContainer


func _init(s, a) -> void:
	state = s
	app = a
	add_theme_constant_override("separation", 0)
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
	_col = _body
	# two columns of cards, one per group, so lines don't stretch across a wide window: the
	# collection and the model on the left, what goes into rekordbox on the right
	var lc := UI.col(16)
	var rc := UI.col(16)
	for c: VBoxContainer in [lc, rc]:
		c.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_body.add_child(UI.row(16, [lc, rc]))
	_side = lc
	_card("Library")
	_add(_path("Music", "music", true))
	_add(_path("rekordbox", "xml", false))
	_add(UI.wrap("Export your rekordbox collection here; vibecheck's playlists go back into it."))
	_add(UI.margin(UI.hline(), 0, 2, 0, 2))
	var total: int = state.library.size()
	var left: int = total - state.embedded
	var now: String = ("now: " + state.listening) if state.listening != "" else ("done" if left == 0 else "paused")
	_add(UI.row(8, [UI.text("Analysed", 12), UI.spacer(),
		UI.text("%s / %s · %s" % [_n(state.embedded), _n(total), now], 12, UI.MUTED, "mono")]))
	var bar := UI.Bar.new(0, float(state.embedded) / maxf(total, 1), UI.LIME)
	bar.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_add(bar)
	_toggle("Analyse the rest in the background (about 5 s a track)", "listen")

	_card("Calibration")
	var th := UI.text("%d labels" % st.threshold, 12, UI.TEXT, "mono")
	_add(UI.row(8, [UI.text("Calibration ends after", 12), UI.spacer(), th]))
	var slider := UI.Track.new(100, 300, 10, st.threshold)
	slider.changed.connect(func(v: float) -> void: th.text = "%d labels" % int(v))
	slider.released.connect(func(v: float) -> void:
		if int(v) != st.threshold:
			state.set_setting("threshold", int(v)))
	_add(slider)
	_add(UI.row(0, [UI.text("100", 10, UI.MUTED, "mono"), UI.spacer(), UI.text("200", 10, UI.MUTED, "mono"), UI.spacer(), UI.text("300", 10, UI.MUTED, "mono")]))
	_add(UI.wrap("Below this the queue brings only calibration tracks, blind and spread across the sound of your collection. Past it, the tracks the model is least sure about, conflicts and old labels, and every tenth one it is sure about, asked blind to check itself."))
	_toggle("Show the model's guess in the player after calibration", "show_guess")
	_add(_line("Re-ask old tracks blind", UI.text("rises as the library fills in", 12, UI.MUTED, "mono")))

	_card("Weighting")
	_toggle("Weigh by stars", "weigh_stars", "5★ ×1 · 1★ ×0.6")
	var rec := UI.row(6)
	for d: Array in [[0, "Off"], [182, "6 months"], [365, "1 year"], [730, "2 years"], [1825, "5 years"]]:
		rec.add_child(UI.chip(d[1], int(st.recency_days) == d[0], func() -> void: state.set_setting("recency_days", d[0])))
	_add(_line("Newer counts more · half after", rec))
	_add(UI.wrap("How much each label and each track's cues count when the models learn: how you made it (kept, confirmed, played) × its stars × its age. Turn stars off if you use them for something else. Older counts less, never below half; labels from before vibecheck have no age."))

	_side = rc
	_card("Rekordbox Export")
	_toggle("Write the colour", "write_colour", "Orange")
	_toggle("Write the star rating", "write_stars", "★★★★")
	_toggle("Write the key into rekordbox's key field", "write_tonality", "8A")
	_toggle("Start the comment with key and energy", "write_comment", "8A - Energy 6")
	_add(UI.wrap("The rest of the comment, /* tags */ included, is kept."))

	_card("Bulk label")
	var sb := UI.text("Label a track when the model is %d%% sure" % int(round(float(st.guesses_sure) * 100)), 12)
	_add(sb)
	var ss := UI.Track.new(60, 95, 5, float(st.guesses_sure) * 100)
	ss.changed.connect(func(v: float) -> void: sb.text = "Label a track when the model is %d%% sure" % int(v))
	ss.released.connect(func(v: float) -> void:
		if int(v) != int(round(float(st.guesses_sure) * 100)):
			state.set_setting("guesses_sure", int(v) / 100.0))
	_add(ss)
	_add(UI.row(0, [UI.text("60%", 10, UI.MUTED, "mono"), UI.spacer(), UI.text("95%", 10, UI.MUTED, "mono")]))
	var rb := UI.text("Unlock when %d%% of blind tests pass" % int(round(float(st.guesses_right) * 100)), 12)
	_add(rb)
	var rs := UI.Track.new(50, 95, 5, float(st.guesses_right) * 100)
	rs.changed.connect(func(v: float) -> void: rb.text = "Unlock when %d%% of blind tests pass" % int(v))
	rs.released.connect(func(v: float) -> void:
		if int(v) != int(round(float(st.guesses_right) * 100)):
			state.set_setting("guesses_right", int(v) / 100.0))
	_add(rs)
	_add(UI.row(0, [UI.text("50%", 10, UI.MUTED, "mono"), UI.spacer(), UI.text("95%", 10, UI.MUTED, "mono")]))
	var meas := UI.row(6)
	for n in [10, 20, 30]:
		meas.add_child(UI.chip(str(n), int(st.guesses_measured) == n, func() -> void: state.set_setting("guesses_measured", n)))
	_add(_line("Unlock after %d blind tests" % int(st.guesses_measured), meas))
	_add(UI.wrap("Adds the model's colour and stars to the export list; Export writes them into rekordbox with key, energy and cues, like your own labels. A guess stays thin trust until you change it in rekordbox or label the track here."))

	_card("Cue points")
	_toggle("Write cue points", "cues_write")
	_toggle("Set cues in the player · Tab", "cues_edit")
	_add(UI.wrap("Tab turns the colour keys into hot cues A–H. Cues you confirm in the player go out with the next Export, in place of the track's cues in rekordbox. Otherwise only tracks with no cues or generated ones (\"Cue 1\", \"Cue 2\", …) get cues; a track with cues of your own keeps them, and Bulk label refreshes vibecheck's own where the model now disagrees."))
	_add(UI.wrap("Where cues go, which loop and how long, and the memory cues are learned from yours: set them in the player or in rekordbox."))

	_end_card()
	var next := "Calibration first: at least %d tracks, blind." % st.threshold if state.calibrating() \
		else "%d labels so far: the queue mixes in what the model is least sure about." % state.labels
	var start := UI.row(8, [UI.wrap(next), UI.button("Start", "ENTER", true, func() -> void: app.show_screen("vibe"))])
	start.get_child(0).size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_body.add_child(UI.margin(start, 0, 10, 0, 0))
	_body.add_child(UI.text(_version(), 10, UI.MUTED, "mono"))


func _n(n: int) -> String:
	var s := str(n)
	var out := ""
	while s.length() > 3:
		out = "," + s.right(3) + out
		s = s.left(s.length() - 3)
	return s + out


func _add(c: Control) -> void:
	_col.add_child(c)


## A group of settings on its own card, titled, with a line on what it is for when that
## isn't plain; _add fills it.
func _card(t: String) -> void:
	_end_card()
	var p := UI.panel(UI.PANEL, 6, 16, 14)
	var s: StyleBoxFlat = p.get_theme_stylebox("panel")
	s.border_color = UI.LINE
	s.set_border_width_all(1)
	_col = UI.col(10)
	p.add_child(_col)
	_side.add_child(p)
	_add(UI.margin(UI.text(t, 18, UI.TEXT, "heavy"), 0, 0, 0, 6))


## No hairline under a card's last line: the card's edge closes it.
func _end_card() -> void:
	if _col == _body or _col.get_child_count() == 0:
		return
	var last := _col.get_child(_col.get_child_count() - 1)
	if last.has_meta("line"):
		(last.get_theme_stylebox("panel") as StyleBoxFlat).border_width_bottom = 0


## A setting on one line, with a hairline under it.
func _line(t: String, right: Control) -> Control:
	var p := UI.section(Color.TRANSPARENT, 6, 0)
	(p.get_theme_stylebox("panel") as StyleBoxFlat).border_color = Color("#34353a")
	p.custom_minimum_size.y = 32
	p.set_meta("line", true)
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
	var p := UI.panel(UI.DARK, 4, 10, 6)
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

