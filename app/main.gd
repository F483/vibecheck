extends Control
## The app: the rail on the left, one screen at a time, and the export and sync dialogs
## dimming everything until confirmed. Design: docs/ui/decisions.md.
##
## Screenshots for docs: godot --path app -- --shot=out.png [--screen=library] [--dialog=export]

const UI := preload("res://ui/ui.gd")
const State := preload("res://ui/state.gd")
const Vibe := preload("res://ui/vibe.gd")
const Library := preload("res://ui/library.gd")
const Stats := preload("res://ui/stats.gd")
const History := preload("res://ui/history.gd")
const Settings := preload("res://ui/settings.gd")
const Player := preload("res://ui/player.gd")
const Status := preload("res://ui/status.gd")
const Filters := preload("res://ui/filters.gd")

const SCREENS := ["vibe", "library", "stats", "history", "settings"]

var state := State.new()
var screens := {}
var current := ""
var dialog: Control = null
## What ENTER does in the open dialog when it is a question, else it closes it.
var dialog_ok := Callable()
var _rail_items := {}
var _status: Control


func _ready() -> void:
	_hidpi()
	var th := Theme.new()
	th.default_font = UI.font("sans")
	th.default_font_size = 12
	theme = th
	var bg := ColorRect.new()
	bg.color = UI.DARK
	bg.set_anchors_preset(Control.PRESET_FULL_RECT)
	add_child(bg)
	var root := UI.row(0)
	root.set_anchors_preset(Control.PRESET_FULL_RECT)
	add_child(root)
	root.add_child(_rail())
	# the status row on top of every screen, then the screen
	var right := UI.col(0)
	right.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	root.add_child(right)
	_status = Status.new(state, self)
	right.add_child(_status)
	var stack := Control.new()
	stack.size_flags_vertical = Control.SIZE_EXPAND_FILL
	right.add_child(stack)
	# one player: it moves to the vibe or library screen, whichever is shown
	var player := Player.new(UI.DARK, state)
	screens = {"vibe": Vibe.new(state, self, player), "library": Library.new(state, self, player), "stats": Stats.new(state),
		"history": History.new(state, self), "settings": Settings.new(state, self)}
	for k: String in SCREENS:
		var s: Control = screens[k]
		s.set_anchors_preset(Control.PRESET_FULL_RECT)
		stack.add_child(s)
	state.failed.connect(_failed)
	state.rekordbox_exported.connect(sync_now)
	get_window().files_dropped.connect(_dropped)
	var watch := Timer.new()
	watch.wait_time = 2.0
	watch.autostart = true
	watch.timeout.connect(state.poll)
	add_child(watch)
	get_tree().auto_accept_quit = false
	for k: String in SCREENS:
		(screens[k] as Control).minimum_size_changed.connect(_fit_min_soon)
	show_screen("vibe" if state.is_ready() else "settings")
	_fit_min_soon()
	state.load_all()
	_shot()


## Closing the window, and any other quit (Ctrl+C in the terminal) on the way out:
## the label writers finish, then the process ends without the engine's teardown.
func _notification(what: int) -> void:
	if what == NOTIFICATION_WM_CLOSE_REQUEST or what == NOTIFICATION_EXIT_TREE:
		state.quit()


## Tracks dragged in from rekordbox (or Finder), on the vibe screen: dropped on the reviewed
## table they join it and the next Export, anywhere else they go to the front of the queue.
func _dropped(paths: PackedStringArray) -> void:
	if not state.loaded or dialog:
		return
	# the drop sends no position; where the mouse is now is where it let go
	var w := get_window()
	var at := Vector2(DisplayServer.mouse_get_position() - w.position) / w.content_scale_factor
	var vibe: Vibe = screens.vibe
	var to_table := current == "vibe" and vibe.is_table(at)
	if not to_table:
		vibe.relisten = false
	var r := state.add_dropped(paths, to_table)
	show_screen("vibe")
	if to_table and not r.tracks.is_empty():
		vibe.open_row(r.tracks[0])
	var missing: PackedStringArray = r.missing
	if missing.is_empty():
		return
	var v := dialog_box(440)
	v.add_child(UI.caps("Not in your collection"))
	v.add_child(UI.wrap("Not added, %d of %d: they aren't tracks in %s." % [missing.size(), paths.size(), state.settings.music], 12, UI.TEXT))
	var names := Array(missing.slice(0, 8))
	if missing.size() > 8:
		names.append("and %d more" % (missing.size() - 8))
	v.add_child(UI.wrap("\n".join(names), 11))
	v.add_child(UI.row(0, [UI.spacer(), UI.button("OK", "ENTER", true, close_dialog)]))


func _failed(text: String) -> void:
	var v := dialog_box(440)
	v.add_child(UI.caps("Something went wrong"))
	v.add_child(UI.wrap(text, 12, UI.TEXT))
	v.add_child(UI.row(0, [UI.spacer(), UI.button("OK", "ENTER", true, close_dialog)]))


func _rail() -> Control:
	var s := UI.style(UI.PANEL)
	s.border_color = UI.LINE
	s.border_width_right = 1
	var p := PanelContainer.new()
	p.add_theme_stylebox_override("panel", UI.pad(s, 6, 10))
	p.custom_minimum_size.x = 64
	var c := UI.col(4)
	for id: String in SCREENS:
		var item := UI.panel(Color.TRANSPARENT, 4, 0, 4)
		item.custom_minimum_size = Vector2(52, 44)
		var icon := UI.Icon.new(id)
		var l := UI.caps(id, UI.MUTED, 9)
		l.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		item.add_child(UI.col(3, [icon, l]))
		UI.clickable(item, show_screen.bind(id))
		_rail_items[id] = [item, icon, l]
		c.add_child(item)
	p.add_child(c)
	return p


func refresh_rail() -> void:
	for id: String in _rail_items:
		var it: Array = _rail_items[id]
		var on := id == current
		var locked := id != "settings" and not state.is_ready()
		(it[0] as PanelContainer).add_theme_stylebox_override("panel", UI.pad(UI.style(UI.LINE if on else Color.TRANSPARENT, 4), 0, 4))
		(it[0] as Control).modulate.a = 0.3 if locked else 1.0
		(it[1] as UI.Icon).color = UI.LIME if on else UI.TEXT
		(it[1] as Control).queue_redraw()
		(it[2] as Label).add_theme_color_override("font_color", UI.LIME if on else UI.MUTED)


func show_screen(id: String) -> void:
	if id != "settings" and not state.is_ready():
		return
	current = id
	for k: String in screens:
		(screens[k] as Control).visible = k == id
	if screens[id].has_method("dock"):
		screens[id].dock()
	refresh_rail()


func open_dialog(body: Control) -> void:
	close_dialog()
	var dim := ColorRect.new()
	dim.color = UI.DIM
	dim.set_anchors_preset(Control.PRESET_FULL_RECT)
	var cc := CenterContainer.new()
	cc.set_anchors_preset(Control.PRESET_FULL_RECT)
	dim.add_child(cc)
	cc.add_child(body)
	add_child(dim)
	dialog = dim


func close_dialog() -> void:
	dialog_ok = Callable()
	if dialog:
		dialog.queue_free()
		dialog = null


func dialog_box(w: float) -> VBoxContainer:
	var p := UI.panel(UI.PANEL, 6, 18, 18)
	var s: StyleBoxFlat = p.get_theme_stylebox("panel")
	s.border_color = Color("#55565c")
	s.set_border_width_all(1)
	p.custom_minimum_size.x = w
	var v := UI.col(9)
	p.add_child(v)
	open_dialog(p)
	return v


func export_batch() -> void:
	if state.reviewed.is_empty():
		_failed("Nothing to export yet: label some tracks first.")
		return
	var v := dialog_box(464)
	v.add_child(UI.caps("Exporting → rekordbox"))
	v.add_child(UI.text("%d tracks" % state.reviewed.size(), 18, UI.TEXT, "heavy"))
	# tracks heard in the player are analysed already; the rest are decoded now, the slow part
	var line := UI.text("Starting…", 12, UI.MUTED, "mono")
	var bar := UI.Bar.new(0, 0.0, UI.LIME)
	v.add_child(UI.wrap("Working out key and energy and placing cue points.", 12))
	v.add_child(bar)
	v.add_child(line)
	var poll := Timer.new()
	poll.wait_time = 0.25
	poll.timeout.connect(func() -> void:
		var p: Vector2i = ClassDB.class_call_static("Vibecheck", "export_progress")
		if p.y == 0:
			return
		bar.frac = float(p.x) / p.y
		bar.queue_redraw()
		line.text = "Analysing audio · %d of %d tracks" % [p.x, p.y] if p.x < p.y else "Writing rekordbox.xml…")
	v.add_child(poll)
	poll.start()
	state.export(func(e: Variant) -> void:
		poll.stop()
		if e == null:
			return
		v = dialog_box(464)
		v.add_child(UI.caps("Exported → rekordbox"))
		v.add_child(UI.text("%d tracks · %s" % [e.count, e.name], 18, UI.TEXT, "heavy"))
		var parts := ["%d colours" % e.coloured, "%d ratings" % e.rated]
		if e.keyed > 0:
			parts.append("%d keys" % e.keyed)
		if e.commented > 0:
			parts.append("%d energy comments" % e.commented)
		if e.cued > 0:
			parts.append("cues on %d" % e.cued)
		if e.not_in_xml > 0:
			parts.append("%d not in the export" % e.not_in_xml)
		v.add_child(UI.text(", ".join(parts), 12, UI.MUTED, "mono"))
		v.add_child(UI.text("written into %s · old one kept as .bak" % state.settings.xml, 11, UI.MUTED, "mono"))
		v.add_child(UI.margin(UI.caps("Now in rekordbox"), 0, 4, 0, 0))
		var steps := [
			"Refresh the rekordbox xml node in the sidebar.",
			"Right-click %s › Import Playlist." % e.name,
			"Play, fix what is wrong, then File › Export Collection in xml format over the same file. vibecheck syncs on its own.",
		]
		for i in steps.size():
			v.add_child(UI.row(8, [UI.text(str(i + 1), 12, UI.LIME, "mono"), UI.wrap(steps[i], 12, UI.TEXT)]))
			(v.get_child(v.get_child_count() - 1).get_child(1) as Control).size_flags_horizontal = Control.SIZE_EXPAND_FILL
		v.add_child(UI.wrap("First time only: Preferences › Advanced › Database › rekordbox xml, choose this file."))
		v.add_child(UI.row(0, [UI.spacer(), UI.button("OK", "ENTER", true, close_dialog)])))


## The model's guesses into rekordbox: what it would write and the guard, then the write.
func export_guesses() -> void:
	if not state.loaded:
		return
	var st: Dictionary = state.settings
	var bar := float(st.guesses_sure)
	var pct := int(round(bar * 100))
	var r: Array = state.right_at(bar)
	var want := state.must_pass(r[1])
	var locked: bool = state.guesses_locked() != ""
	var v := dialog_box(464)
	v.add_child(UI.caps("Bulk label → export list"))
	var n := state.sure_at(bar, true)
	var lim := int(st.guesses_limit)
	v.add_child(UI.text("%s of %s tracks" % [UI.thousands(lim), UI.thousands(n)] if lim > 0 and n > lim else "%s tracks" % UI.thousands(n), 18, UI.TEXT, "heavy"))
	v.add_child(UI.text("no colour yet, the model %d%% sure of one%s" % [pct, ", surest first" if lim > 0 and n > lim else ""], 12, UI.MUTED, "mono"))
	if not locked:
		# how many per run; kept in [guesses] limit, so the next run starts from it
		var lr := UI.row(6, [UI.text("Label at most", 12), UI.spacer()])
		for c in [50, 100, 250, 0]:
			lr.add_child(UI.chip("All" if c == 0 else str(c), lim == c, func() -> void:
				state.set_setting("guesses_limit", c)
				export_guesses()))
		v.add_child(lr)
	# the three conditions, in the words of Settings › Bulk label, each with where it stands
	var checks := UI.col(6)
	var li := UI.Icon.new("lock" if locked else "unlock")
	li.custom_minimum_size = Vector2(14, 14)
	li.color = UI.AMBER if locked else UI.DECIDE
	checks.add_child(UI.row(8, [li, UI.caps("Locked" if locked else "Unlocked", li.color)]))
	var cal := not state.calibrating()
	_check(checks, cal, "Calibration done", "%s of %s labels" % [UI.thousands(mini(state.labels, int(st.threshold))), UI.thousands(int(st.threshold))])
	_check(checks, r[0] >= want, "Blind tests passed", "%d of %d · needs %d" % [r[0], r[1], want])
	var box := UI.panel(UI.DARK, 4, 12, 10)
	box.add_child(checks)
	v.add_child(box)
	v.add_child(UI.wrap("Adds the model's colour and stars to the export list, surest first; Export then writes them into rekordbox like your own labels. A guess stays thin trust (not trained on) until you change it in rekordbox or label the track here. A blind test is a track the model is %d%% sure of, asked in the queue without showing its guess (every 10th card). Settings › Bulk label sets the unlock rule." % pct))
	if locked:
		v.add_child(UI.row(0, [UI.spacer(), UI.button("OK", "ENTER", true, close_dialog)]))
		return
	v.add_child(UI.row(8, [UI.spacer(), UI.button("Cancel", "ESC", false, close_dialog), UI.button("Label", "ENTER", true, _write_guesses)]))
	dialog_ok = _write_guesses


## One unlock condition: a tick or a cross, its name, and where it stands.
func _check(into: Control, ok: bool, what: String, status: String) -> void:
	into.add_child(UI.row(8, [UI.text("✓" if ok else "✗", 12, UI.DECIDE if ok else UI.AMBER, "mono"), UI.text(what, 12),
		UI.spacer(), UI.text(status, 12, UI.TEXT if ok else UI.AMBER, "mono")]))


func _write_guesses() -> void:
	var v := dialog_box(464)
	v.add_child(UI.caps("Bulk label → export list"))
	v.add_child(UI.wrap("A moment: the whole collection is scored.", 12))
	state.bulk_label(func(e: Variant) -> void:
		if e == null:
			return
		v = dialog_box(464)
		v.add_child(UI.caps("Bulk labelled → export list"))
		v.add_child(UI.text("%s tracks" % UI.thousands(e.count), 18, UI.TEXT, "heavy"))
		v.add_child(UI.text("%d with stars · on the export list now" % e.rated, 12, UI.MUTED, "mono"))
		v.add_child(UI.wrap("Check them in the list below, then Export (X) writes them into rekordbox with key, energy and cues, like the labels you made in the queue.", 12, UI.TEXT))
		v.add_child(UI.row(0, [UI.spacer(), UI.button("OK", "ENTER", true, close_dialog)])))
	dialog_ok = Callable()


func sync_now() -> void:
	if not state.loaded:
		return
	var v := dialog_box(400)
	v.add_child(UI.caps("Reading rekordbox.xml…"))
	state.sync(func(r: Variant) -> void:
		if r == null:
			return
		v = dialog_box(420)
		v.add_child(UI.caps("rekordbox.xml · synced"))
		# played: anywhere in the collection, whichever export the label went out in
		var line := func(ring: int, k: int, what: String, hint: String) -> void:
			var n := UI.text(str(k), 18, UI.TEXT if ring > 1 else UI.MUTED, "mono")
			n.custom_minimum_size.x = 40
			v.add_child(UI.row(10, [UI.Ring.new(ring), n, UI.text(what, 12), UI.spacer(), UI.text(hint, 12, UI.MUTED, "mono")]))
		line.call(4, int(r.played), "played", "counts most")
		v.add_child(UI.text("Last export, %d tracks" % r.out if r.out > 0 else "No export from here yet", 12, UI.MUTED, "mono"))
		if r.out > 0:
			line.call(3, int(r.corrected), "corrected", "now confirmed")
			line.call(1, int(r.untouched), "untouched", "still guessed")
		var more := []
		if r.added + r.changed > r.corrected:
			more.append("%d more labels taken from rekordbox" % (r.added + r.changed - r.corrected))
		if r.confirmed > 0:
			more.append("%d from the command line's rounds confirmed" % r.confirmed)
		if r.newer > 0:
			more.append("%d left alone: labelled here, not exported yet" % r.newer)
		if r.unknown > 0:
			more.append("%d colours from before vibecheck ignored" % r.unknown)
		if not more.is_empty():
			v.add_child(UI.wrap(", ".join(more) + ".", 11))
		v.add_child(UI.wrap("Your queue is being rebuilt: conflicts, old labels and unsure tracks.", 12))
		v.add_child(UI.row(0, [UI.spacer(), UI.button("Back to vibe", "ENTER", true, close_dialog)])))


func _unhandled_key_input(e: InputEvent) -> void:
	var k := e as InputEventKey
	if not k or not k.pressed:
		return
	if dialog:
		if k.keycode in [KEY_ENTER, KEY_KP_ENTER] and dialog_ok.is_valid():
			var ok := dialog_ok
			close_dialog()
			ok.call()
		elif k.keycode in [KEY_ENTER, KEY_KP_ENTER, KEY_ESCAPE]:
			close_dialog()
		get_viewport().set_input_as_handled()
		return
	if k.echo and not k.keycode in [KEY_UP, KEY_DOWN, KEY_LEFT, KEY_RIGHT]:
		return
	if k.alt_pressed and k.keycode == KEY_S:
		sync_now()
	elif current == "vibe" and k.keycode == KEY_X and not k.alt_pressed and not UI.is_colour_key(k):
		export_batch()
	elif current == "settings" and k.keycode in [KEY_ENTER, KEY_KP_ENTER]:
		show_screen("vibe")
	elif screens[current].has_method("handle_key") and screens[current].handle_key(k):
		pass
	else:
		return
	get_viewport().set_input_as_handled()


## The window can't be made narrower or shorter than the largest screen needs, so nothing is
## ever pushed out of view. Measured from the screens, so it follows the layout.
## Measured a frame later: rows rebuilt this frame still hold their old children until it ends,
## and would count twice.
var _fit_pending := false


func _fit_min_soon() -> void:
	if _fit_pending:
		return
	_fit_pending = true
	await get_tree().process_frame
	_fit_pending = false
	_fit_min()


func _fit_min() -> void:
	var need := Vector2.ZERO
	for k: String in SCREENS:
		need = need.max((screens[k] as Control).get_combined_minimum_size())
	need.x += 64  # the rail
	need.y += _status.get_combined_minimum_size().y
	var w := get_window()
	var px := Vector2i((need * w.content_scale_factor).ceil())
	if px != w.min_size:
		w.min_size = px


## Retina reports pixels, so a 1280 px window shows at half size: scale window and content by the screen.
func _hidpi() -> void:
	var w := get_window()
	var s := DisplayServer.screen_get_scale(w.current_screen)
	if s <= 1.0:
		return
	w.content_scale_factor = s
	w.size = Vector2i(Vector2(w.size) * s)
	w.move_to_center()


## --shot=path.png [--screen=id] [--dialog=export|sync|batch|why|cues|tags|filter:colours|energy|…] [--wait=s] [--after=s]: once loaded, wait,
## show the screen, wait again, render one frame to a file and quit.
func _shot() -> void:
	var args := {}
	for a in OS.get_cmdline_user_args():
		var kv := a.trim_prefix("--").split("=", true, 1)
		args[kv[0]] = kv[1] if kv.size() > 1 else ""
	if not args.has("shot"):
		return
	var t := Time.get_ticks_msec()
	while not state.loaded and Time.get_ticks_msec() - t < 120000:
		await get_tree().process_frame
	await get_tree().create_timer(float(args.get("wait", "1"))).timeout
	show_screen(args.get("screen", "vibe"))
	await get_tree().create_timer(float(args.get("after", "0"))).timeout
	match args.get("dialog", ""):
		"export":
			export_batch()
		"guesses":
			export_guesses()
		"sync":
			sync_now()
		"cues":
			# cue mode on the track playing, its hot cue D selected
			var pl = screens.vibe.player
			while not pl._cues_ready and Time.get_ticks_msec() - t < 180000:
				await get_tree().process_frame
			pl.set_cue_mode(true)
			pl.cue_sel = pl._hot(3)
			pl._cues_changed()
		"tags":
			var pt = screens.vibe.player
			while not pt._tags_ready and Time.get_ticks_msec() - t < 180000:
				await get_tree().process_frame
			pt.set_view("tags")
		"why":
			screens.vibe.player.open_why()
			await get_tree().create_timer(0.5).timeout
		"batch":
			(screens.history as History).open_newest()
			await get_tree().create_timer(1.0).timeout
		var d when d.begins_with("filter:"):
			(screens[args.get("screen", "library")].filters as Filters).open_filter(d.trim_prefix("filter:"))
			await get_tree().create_timer(0.5).timeout
	for i in 8:
		await get_tree().process_frame
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png(args.shot)
	state.stop()
	get_tree().quit()
