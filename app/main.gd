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
const Settings := preload("res://ui/settings.gd")

const SCREENS := ["vibe", "library", "stats", "settings"]

var state := State.new()
var screens := {}
var current := ""
var dialog: Control = null
var _rail_items := {}


func _ready() -> void:
	_hidpi()
	var th := Theme.new()
	th.default_font = UI.font("sans")
	th.default_font_size = 12
	theme = th
	var bg := ColorRect.new()
	bg.color = UI.PANEL
	bg.set_anchors_preset(Control.PRESET_FULL_RECT)
	add_child(bg)
	var root := UI.row(0)
	root.set_anchors_preset(Control.PRESET_FULL_RECT)
	add_child(root)
	root.add_child(_rail())
	var stack := Control.new()
	stack.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	root.add_child(stack)
	screens = {"vibe": Vibe.new(state, self), "library": Library.new(state, self), "stats": Stats.new(state),
		"settings": Settings.new(state, self)}
	for k: String in SCREENS:
		var s: Control = screens[k]
		s.set_anchors_preset(Control.PRESET_FULL_RECT)
		stack.add_child(s)
	show_screen("vibe" if state.is_ready() else "settings")
	_shot()


func _rail() -> Control:
	var s := UI.style(UI.DARK)
	s.border_color = UI.LINE
	s.border_width_right = 1
	var p := PanelContainer.new()
	p.add_theme_stylebox_override("panel", UI.pad(s, 6, 10))
	p.custom_minimum_size.x = 64
	var c := UI.col(4)
	var logo := UI.panel(UI.LIME, 6)
	logo.custom_minimum_size = Vector2(32, 32)
	logo.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	var v := UI.text("v", 15, UI.BG, "heavy")
	v.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	logo.add_child(v)
	c.add_child(UI.margin(logo, 0, 0, 0, 8))
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
	if dialog:
		dialog.queue_free()
		dialog = null


func _dialog_box(w: float) -> VBoxContainer:
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
	state.record_export()
	var e: Dictionary = state.last_export
	var v := _dialog_box(464)
	v.add_child(UI.caps("Exported → rekordbox"))
	v.add_child(UI.text("%d tracks · %s" % [e.count, e.name], 18, UI.TEXT, "heavy"))
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
	v.add_child(UI.row(0, [UI.spacer(), UI.button("OK", "ENTER", true, close_dialog)]))


func sync_now() -> void:
	state.apply_sync()
	var r: Dictionary = state.last_sync
	var v := _dialog_box(400)
	v.add_child(UI.caps("rekordbox.xml changed · synced"))
	v.add_child(UI.text("Batch %s" % Time.get_date_string_from_system(), 18, UI.TEXT, "heavy"))
	for line: Array in [[4, r.played, "played", "counts most"], [3, r.corrected, "corrected", "set by hand"],
			[1, r.untouched, "untouched", "counts a little"]]:
		var n := UI.text(str(line[1]), 18, UI.TEXT if line[0] > 1 else UI.MUTED, "mono")
		n.custom_minimum_size.x = 40
		v.add_child(UI.row(10, [UI.Ring.new(line[0]), n, UI.text(line[2], 12), UI.spacer(), UI.text(line[3], 12, UI.MUTED, "mono")]))
	v.add_child(UI.wrap("%d tracks went to the front of your queue: conflicts, old labels and unsure ones." % r.queued, 12))
	v.add_child(UI.row(0, [UI.spacer(), UI.button("Back to vibe", "ENTER", true, close_dialog)]))


func _unhandled_key_input(e: InputEvent) -> void:
	var k := e as InputEventKey
	if not k or not k.pressed:
		return
	if dialog:
		if k.keycode in [KEY_ENTER, KEY_KP_ENTER, KEY_ESCAPE]:
			close_dialog()
		get_viewport().set_input_as_handled()
		return
	if k.echo and not k.keycode in [KEY_UP, KEY_DOWN, KEY_LEFT, KEY_RIGHT]:
		return
	if k.alt_pressed and k.physical_keycode == KEY_S:
		sync_now()
	elif current == "vibe" and k.physical_keycode == KEY_X and not k.alt_pressed:
		export_batch()
	elif current == "settings" and k.keycode in [KEY_ENTER, KEY_KP_ENTER]:
		show_screen("vibe")
	elif screens[current].has_method("handle_key") and screens[current].handle_key(k):
		pass
	else:
		return
	get_viewport().set_input_as_handled()


## Retina reports pixels, so a 1280 px window shows at half size: scale window and content by the screen.
func _hidpi() -> void:
	var w := get_window()
	var s := DisplayServer.screen_get_scale(w.current_screen)
	if s <= 1.0:
		return
	w.content_scale_factor = s
	w.size = Vector2i(Vector2(w.size) * s)
	w.move_to_center()


## --shot=path.png [--screen=id] [--dialog=export|sync]: render one frame to a file and quit.
func _shot() -> void:
	var args := {}
	for a in OS.get_cmdline_user_args():
		var kv := a.trim_prefix("--").split("=", true, 1)
		args[kv[0]] = kv[1] if kv.size() > 1 else ""
	if not args.has("shot"):
		return
	show_screen(args.get("screen", "vibe"))
	match args.get("dialog", ""):
		"export":
			export_batch()
		"sync":
			sync_now()
	for i in 8:
		await get_tree().process_frame
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png(args.shot)
	get_tree().quit()
