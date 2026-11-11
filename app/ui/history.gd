extends VBoxContainer
## What came in from rekordbox (left) and went out to it (right), newest first. A card is a
## sync or an export; clicking one lists its tracks. For looking back only: nothing here
## changes a label.

const UI := preload("res://ui/ui.gd")

var state
var app
var _in: VBoxContainer
var _out: VBoxContainer
var _n_in: Label
var _n_out: Label
var _newest := {}


func _init(s, a) -> void:
	state = s
	app = a
	add_theme_constant_override("separation", 0)
	_in = UI.col(8)
	_out = UI.col(8)
	_n_in = UI.text("", 11, UI.MUTED, "mono")
	_n_out = UI.text("", 11, UI.MUTED, "mono")
	var cols := UI.row(16, [
		_side(_in, _n_in, "In from rekordbox", "Each sync: what you changed in rekordbox, and what you played."),
		_side(_out, _n_out, "Out to rekordbox", "Each export, and what became of its tracks since.")])
	var sc := ScrollContainer.new()
	sc.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	sc.size_flags_vertical = Control.SIZE_EXPAND_FILL
	var m := UI.margin(cols, 16, 12, 16, 16)
	m.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	sc.add_child(m)
	add_child(sc)
	visibility_changed.connect(reload)
	state.changed.connect(func() -> void:
		if is_visible_in_tree():
			reload())


## A column: its title as the screen's title (no title row: the status row is on top), with
## how many cards it holds right of it.
func _side(list: VBoxContainer, count: Label, title: String, about: String) -> Control:
	var c := UI.col(8, [UI.row(10, [UI.text(title, 15, UI.TEXT, "semi"), count]), UI.wrap(about), list])
	c.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	c.size_flags_stretch_ratio = 1.0
	return c


func reload() -> void:
	if not is_visible_in_tree() or not state.loaded:
		return
	state.load_history(_show)


func _show(h: Array) -> void:
	for l: VBoxContainer in [_in, _out]:
		for c in l.get_children():
			c.queue_free()
	var ins := h.filter(func(b: Dictionary) -> bool: return b.dir == "in")
	var outs := h.filter(func(b: Dictionary) -> bool: return b.dir == "out")
	_n_in.text = "%d sync%s" % [ins.size(), "" if ins.size() == 1 else "s"]
	_n_out.text = "%d export%s" % [outs.size(), "" if outs.size() == 1 else "s"]
	_newest = h[0] if not h.is_empty() else {}
	# syncs that changed nothing fold into one line per run of them
	var quiet: Array = []
	for b: Dictionary in ins:
		if _changed(b):
			_fold(quiet)
			quiet = []
			_in.add_child(_card_in(b))
		else:
			quiet.append(b)
	_fold(quiet)
	for b: Dictionary in outs:
		_out.add_child(_card_out(b))
	for l: VBoxContainer in [_in, _out]:
		if l.get_child_count() == 0:
			l.add_child(UI.text("nothing yet", 12, UI.MUTED))


func _changed(b: Dictionary) -> bool:
	var c: Dictionary = b.counts
	return int(c.get("added", 0)) + int(c.get("changed", 0)) + int(c.get("played", 0)) + int(c.get("confirmed", 0)) > 0


func _fold(quiet: Array) -> void:
	if quiet.is_empty():
		return
	var span := _when(quiet[-1].ts) if quiet.size() == 1 else "%s – %s" % [_when(quiet[-1].ts), _when(quiet[0].ts)]
	var t := "1 sync, no changes" if quiet.size() == 1 else "%d syncs, no changes" % quiet.size()
	_in.add_child(UI.margin(UI.row(8, [UI.text(t, 11, UI.MUTED), UI.spacer(), UI.text(span, 11, UI.MUTED, "mono")]), 12, 0, 12, 0))


func _card(top: Array, lines: Array, b: Dictionary) -> Control:
	var p := UI.panel(UI.PANEL, 4, 12, 10)
	var c := UI.col(6, [UI.row(8, top)])
	for l: Array in lines:
		if not l.is_empty():
			c.add_child(UI.row(10, l))
	p.add_child(c)
	UI.clickable(p, _open.bind(b))
	return p


func _card_in(b: Dictionary) -> Control:
	var c: Dictionary = b.counts
	var corrected := int(c.changed)
	var line: Array = []
	for it: Array in [[3, corrected, "corrected"], [4, int(c.played), "played"], [2, int(c.added), "added"]]:
		if it[1] > 0:
			line += [UI.Ring.new(it[0]), UI.text("%d %s" % [it[1], it[2]], 12, UI.TEXT)]
	var more: Array = []
	if int(c.get("confirmed", 0)) > 0:
		more.append("%d confirmed" % int(c.confirmed))
	if int(c.get("newer", 0)) > 0:
		more.append("%d left alone, not exported yet" % int(c.newer))
	var xml := "rekordbox exported it %s" % _when(int(b.xml_ts)) if b.xml_ts != null else str(b.name)
	return _card([UI.text(_when(int(b.ts)), 13, UI.TEXT, "semi"), UI.spacer(), UI.text(xml, 11, UI.MUTED, "mono")],
		[line, [UI.text(", ".join(more), 11, UI.MUTED)] if not more.is_empty() else []], b)


func _card_out(b: Dictionary) -> Control:
	var c: Dictionary = b.counts
	var wrote: Array = ["%d tracks" % int(b.tracks)]
	if int(c.get("keyed", 0)) > 0:
		wrote.append("%d keys" % int(c.keyed))
	if int(c.get("commented", 0)) > 0:
		wrote.append("%d energy comments" % int(c.commented))
	if int(c.get("cued", 0)) > 0:
		wrote.append("cues on %d" % int(c.cued))
	var fate: Array = []
	if int(b.played) + int(b.corrected) + int(b.untouched) == 0:
		fate = [UI.text("not synced since", 11, UI.MUTED)]
	else:
		for it: Array in [[4, int(b.played), "played"], [3, int(b.corrected), "corrected"], [1, int(b.untouched), "untouched"]]:
			fate += [UI.Ring.new(it[0]), UI.text("%d %s" % [it[1], it[2]], 12, UI.TEXT if it[0] > 1 else UI.MUTED)]
	return _card([UI.text(str(b.name), 13, UI.TEXT, "semi"), UI.spacer(), UI.text(_when(int(b.ts)), 11, UI.MUTED, "mono")],
		[[UI.text(" · ".join(wrote), 12, UI.MUTED, "mono")], fate], b)


## The newest batch's list, for screenshots (--dialog=batch).
func open_newest() -> void:
	if not _newest.is_empty():
		_open(_newest)


## Its tracks, in a dialog over the page.
func _open(b: Dictionary) -> void:
	state.load_batch(int(b.id), func(ts: Array) -> void:
		var v: VBoxContainer = app.dialog_box(720)
		var in_: bool = b.dir == "in"
		v.add_child(UI.caps("Sync · from rekordbox" if in_ else "Export · to rekordbox"))
		v.add_child(UI.text(_when(int(b.ts)) if in_ else "%s · %s" % [b.name, _when(int(b.ts))], 18, UI.TEXT, "heavy"))
		var list := UI.col(4)
		ts.sort_custom(func(x: Dictionary, y: Dictionary) -> bool: return _title(x.path) < _title(y.path))
		for t: Dictionary in ts:
			list.add_child(_track(t, in_))
		if ts.is_empty():
			list.add_child(UI.text("no tracks kept for this one", 12, UI.MUTED))
		var sc := ScrollContainer.new()
		sc.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
		sc.custom_minimum_size.y = mini(ts.size() * 23, 420)
		list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		sc.add_child(list)
		v.add_child(sc)
		v.add_child(UI.row(0, [UI.spacer(), UI.button("Close", "ENTER", true, app.close_dialog)])))


func _track(t: Dictionary, in_: bool) -> Control:
	var name := UI.text(_title(t.path), 12, UI.TEXT)
	name.clip_text = true
	name.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	var r := UI.row(10)
	if in_:
		var what := UI.text(str(t.what), 10, UI.MUTED, "mono")
		what.custom_minimum_size.x = 64
		r.add_child(what)
		if t.what == "corrected":
			r.add_child(_label(t.colour_before, t.stars_before, true))
			r.add_child(UI.text("→", 12, UI.MUTED))
	r.add_child(_label(t.colour, t.stars))
	r.add_child(name)
	if not in_:
		var extra: Array = []
		if t.key != null:
			extra.append(str(t.key))
		if t.cues != null and int(t.cues) > 0:
			extra.append("%d cues" % int(t.cues))
		r.add_child(UI.text("  ".join(extra), 11, UI.MUTED, "mono"))
		var f := str(t.fate) if t.fate != null else ""
		var fr := UI.row(4, [UI.Ring.new({"played": 4, "corrected": 3}.get(f, 1)), UI.text(f if f != "" else "—", 11, UI.MUTED)])
		fr.custom_minimum_size.x = 84
		r.add_child(fr if f != "" else UI.text("", 11))
	return r


## A colour swatch with its name and the stars, as the table shows them.
func _label(colour: Variant, stars: Variant, dim := false) -> Control:
	var i: int = state._ui_of.get(int(colour), -1) if colour != null else -1
	var n := UI.text(UI.COLOURS[i].name if i >= 0 else "none", 12, UI.MUTED if dim or i < 0 else UI.TEXT)
	n.custom_minimum_size.x = 50
	var s := UI.text("★".repeat(maxi(int(stars), 0)) if stars != null else "", 11, UI.MUTED if dim else UI.TEXT)
	s.custom_minimum_size.x = 56
	return UI.row(6, [UI.rect(UI.COLOURS[i].c if i >= 0 else UI.LINE, 6, 14), n, s])


func _title(path: String) -> String:
	var t: Dictionary = state.by_path.get(path, {})
	if t.is_empty():
		return path.get_file().get_basename()
	return "%s — %s" % [t.title, t.artist] if str(t.artist) != "" else str(t.title)


## "today 06:05", "yesterday 21:40", "8 Nov 21:40", in local time.
func _when(ts: int) -> String:
	var bias := int(Time.get_time_zone_from_system().bias) * 60
	var d := Time.get_datetime_dict_from_unix_time(ts + bias)
	var now := int(Time.get_unix_time_from_system()) + bias
	var hm := "%02d:%02d" % [d.hour, d.minute]
	var days := int(now / 86400) - int((ts + bias) / 86400)
	if days == 0:
		return "today " + hm
	if days == 1:
		return "yesterday " + hm
	var months := ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
	return "%d %s %s" % [d.day, months[d.month - 1], hm]
