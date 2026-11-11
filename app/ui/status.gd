extends PanelContainer
## The status row on top of every screen: what the model is doing in the background (left),
## and where the tracks come from, rekordbox and how fresh its export is (right).

const UI := preload("res://ui/ui.gd")

var state
var app
var _row: HBoxContainer


func _init(s, a) -> void:
	state = s
	app = a
	var st := StyleBoxFlat.new()
	st.bg_color = UI.PANEL
	st.border_color = UI.LINE
	st.border_width_bottom = 1
	add_theme_stylebox_override("panel", UI.pad(st, 16, 8))
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	_row = UI.row(8)
	add_child(_row)
	state.changed.connect(refresh)
	refresh()


func refresh() -> void:
	for c in _row.get_children():
		c.queue_free()
	var parts: Array = [UI.caps("Model")]
	if not state.loaded:
		parts += [UI.text("reading your labels…", 12, UI.MUTED)]
	else:
		# what it is doing in the background, as the console has it
		var s: Array = state.listening_status()
		parts += [UI.text(s[0], 12, UI.TEXT)]
		if s[1] < 1.0:
			parts += [UI.Bar.new(160, s[1], UI.LIME)]
		parts += [UI.text(s[2], 12, UI.MUTED, "mono")]
	parts += [UI.spacer(), UI.caps("rekordbox")]
	if str(state.settings.xml) == "":
		parts += [UI.text("not set", 11, UI.MUTED)]
		for c: Control in parts:
			_row.add_child(c)
		return
	var old: bool = state.export_old()
	var sync := UI.panel(UI.LINE, 4, 6, 1)
	sync.add_child(UI.row(4, [UI.text("Sync", 11, UI.TEXT, "bold"), UI.kbd("⌥S")]))
	UI.clickable(sync, Callable(app, "sync_now"))
	parts += [UI.rect(UI.LIME, 6, 6), UI.text("watching " + str(state.settings.xml).get_file(), 11, UI.MUTED),
		UI.text("·", 11, UI.MUTED), UI.text("collection exported " + _ago(state.export_age_days()), 11, UI.AMBER if old else UI.TEXT)]
	if old:
		parts += [UI.text("⚠ old: export it again in rekordbox before Export", 11, UI.AMBER)]
	parts += [UI.margin(sync, 6, 0, 0, 0)]
	for c: Control in parts:
		_row.add_child(c)


func _ago(days: int) -> String:
	match days:
		-1:
			return "not yet"
		0:
			return "today"
		1:
			return "yesterday"
	return "%d d ago" % days
