extends RefCounted
## App state, from the Rust core (vibecheck::app, through the extension's one JSON call).
##
## Calls run in order on worker threads and answer on the main thread, so the UI never
## waits: one lane for the library, queue and decisions, one for waveforms, one for
## listening to new tracks in the background. Settings of the app itself live in
## user://settings.cfg; those of the collection in its .vibecheck/config.toml, through the core.

signal changed
signal status(text: String)
signal failed(text: String)
signal rekordbox_exported

const UI := preload("res://ui/ui.gd")
const CFG := "user://settings.cfg"
## rekordbox's colours by palette id (1..8), as the core numbers them
const PALETTE := ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Pink"]
## settings the core keeps, in the collection's config.toml: key -> [section, key]
const CORE_KEYS := {
	"write_tonality": ["rekordbox", "write_tonality"], "write_comment": ["rekordbox", "write_comment"],
	"cues_write": ["cues", "write"], "cues_lead": ["cues", "lead"], "cues_loops": ["cues", "loops"],
	"cues_memory": ["cues", "memory"],
}

var library: Array[Dictionary] = []
var by_path := {}
var queue: Array[Dictionary] = []
var reviewed: Array[Dictionary] = []
var labels := 0
var measured := 0
var pool := 0
var loaded := false
var embedded := 0
var exported_at := 0
var queued_by_sync := 0
var guessing := 0  # tracks the model is listening to and guessing now: their rings spin
var last_export := {}
var last_sync := {}
var stats := {}
var listening := ""
var _hearing := false  # the background listening chain is running
var settings := {
	"music": "", "xml": "~/Documents/rekordbox.xml", "threshold": 100, "show_guess": true, "warn_days": 7,
	"listen": true, "write_tonality": true, "write_comment": true, "cues_write": true, "cues_lead": 0,
	"cues_loops": {}, "cues_memory": true,
}

var _acc := Vector2(0, 100)
var _main := Lane.new()
var _waves := Lane.new()
var _stats := Lane.new()
var _bg := Lane.new()
var _model := Lane.new()  # the first queue and retraining, beside the library loading
var _library_ready := false
var _pending_queue := Callable()
var _ask := Lane.new()  # the model's guesses for dropped tracks: not behind background listening
var _answered := {}
var _skipped: Array[Dictionary] = []
var _front := {}  # paths you put at the front of the queue: kept there when it is rebuilt
var _covers := {}
var _per_track := 0.0  # seconds background listening takes a track, smoothed
var _said_at := -1  # when the console last got the listening line, msec
## Waveforms and parts of the player's track and the ones it may load next, as they come:
## path -> {"wave": …, "parts": …}. Others are forgotten; the core keeps them in its cache.
var _wv := {}
var _wv_now := ""
var _wv_cb := {}
var _seen_mtime := 0
var _exporting := false
var _decided := 0
var _saving := true
var _ui_of := {}


func _init() -> void:
	for i in UI.COLOURS.size():
		_ui_of[PALETTE.find(UI.COLOURS[i].name) + 1] = i
	var f := ConfigFile.new()
	# the file can hold only the rekordbox section, before any setting is saved
	if f.load(CFG) == OK and f.has_section("app"):
		for k: String in f.get_section_keys("app"):
			settings[k] = f.get_value("app", k)
	# not chosen yet: the usual folder, if it has been used. Also with other settings saved,
	# or saving any of them would lose it
	if settings.music == "" and DirAccess.dir_exists_absolute(_abs("~/Music/Collection/.vibecheck")):
		settings.music = "~/Music/Collection"
	# --music= and --xml= try other files without touching the saved settings
	for a in OS.get_cmdline_user_args():
		for k in ["music", "xml"]:
			if a.begins_with("--%s=" % k):
				settings[k] = a.split("=", true, 1)[1]
				_saving = false


## Close down: long work in the core gives up at its next safe point, then the
## workers are joined and what holds textures let go, so nothing leaks at exit.
## Closing the app: long work stops at its next safe point; wait only for the lanes that write
## labels (quick now), then end the process at once, without waiting for background listening
## (cache writes only: an interrupted one rolls back) or the engine's teardown.
func quit() -> void:
	ClassDB.class_call_static("Vibecheck", "stop")
	for l: Lane in [_main, _ask, _model]:
		l.stop()
	ClassDB.class_call_static("Vibecheck", "exit")


func stop() -> void:
	ClassDB.class_call_static("Vibecheck", "stop")
	for l: Lane in [_main, _waves, _stats, _bg, _ask, _model]:
		l.stop()
	_covers.clear()
	_wv.clear()
	queue.clear()
	reviewed.clear()
	library.clear()
	by_path.clear()


func _abs(p: String) -> String:
	return OS.get_environment("HOME") + p.substr(1) if p.begins_with("~/") else p


func is_ready() -> bool:
	return settings.music != "" and settings.xml != "" and DirAccess.dir_exists_absolute(_abs(settings.music))


func calibrating() -> bool:
	return labels < settings.threshold


## How often the model in use picks your colour, as a 95% range, in percent.
func accuracy() -> Vector2i:
	return Vector2i(roundi(_acc.x), roundi(_acc.y))


func export_age_days() -> int:
	return int((Time.get_unix_time_from_system() - exported_at) / 86400.0) if exported_at > 0 else -1


func export_old() -> bool:
	return export_age_days() > settings.warn_days


func busy() -> bool:
	return _main.busy > 0


func _call(lane: Lane, cmd: String, args: Dictionary, done: Callable, what := "") -> void:
	args.root = settings.music
	args.xml = settings.xml
	if what != "":
		status.emit(what)
	lane.push(cmd, JSON.stringify(args), str(args.get("path", "")), func(r: Variant) -> void:
		if what != "":
			status.emit("")
		if not r is Dictionary or r.has("error"):
			failed.emit(str(r.error) if r is Dictionary else "%s failed" % cmd)
			done.call(null)
		else:
			done.call(r.ok))


# --- loading ---------------------------------------------------------------------------

## Scan the folder, read the library, then build the queue.
func load_all() -> void:
	if not is_ready():
		return
	loaded = false
	_call(_main, "open", {}, func(o: Variant) -> void:
		if o == null:
			return
		var m := FileAccess.get_modified_time(_abs(settings.xml))
		_seen_mtime = m
		var f := ConfigFile.new()
		f.load(CFG)
		exported_at = f.get_value("rekordbox", settings.xml, m)
		# the library and the first queue load side by side; the queue waits for the library
		_library_ready = false
		_load_library(func() -> void:
			_library_ready = true
			if _pending_queue.is_valid():
				var p := _pending_queue
				_pending_queue = Callable()
				p.call())
		refresh_queue(false, true), "Reading your collection…")
	_call(_main, "settings", {}, func(c: Variant) -> void:
		if c == null:
			return
		for k: String in c:
			settings[k] = c[k]
		settings.cues_lead = 1 if Array(c.cues_lead).size() > 1 else 0
		for s: String in settings.cues_loops:
			settings.cues_loops[s] = int(settings.cues_loops[s])
		changed.emit())


func _load_library(then := Callable()) -> void:
	_call(_main, "library", {}, func(rows: Variant) -> void:
		if rows == null:
			return
		var old := by_path
		library.clear()
		by_path = {}
		reviewed.clear()
		embedded = 0
		for r: Dictionary in rows:
			var t := _row(r)
			# keep what the session knows: flags, queue tags
			if old.has(t.path):
				var o: Dictionary = old[t.path]
				t.flag = o.flag
				t.tag = o.tag
			library.append(t)
			by_path[t.path] = t
			embedded += int(t.embedded)
			if t.batch:
				reviewed.append(t)
		reviewed.sort_custom(func(a: Dictionary, b: Dictionary) -> bool: return a.labelled_ts > b.labelled_ts)
		# the queue holds the same dictionaries
		for i in queue.size():
			queue[i] = by_path.get(queue[i].path, queue[i])
		changed.emit()
		if then.is_valid():
			then.call()
		_listen(), "Reading your collection…")


func _row(r: Dictionary) -> Dictionary:
	var ts := int(r.labelled)
	return {
		"path": r.path, "title": r.title, "artist": r.artist, "mix": r.mix,
		"colour": _ui_of.get(int(r.colour), -1), "rating": maxi(int(r.stars), 0), "source": r.source,
		"trust": int(r.trust), "energy": int(r.energy), "key": r.key, "bpm": float(r.bpm), "plays": int(r.plays),
		"added": r.added, "labelled": Time.get_date_string_from_unix_time(ts) if ts > 0 else "",
		"labelled_ts": ts, "batch": r.batch, "joined": int(r.joined), "embedded": r.embedded, "length": maxf(float(r.length), 1.0),
		"tag": "", "flag": "", "guess": -1, "guess_stars": -1,
		"cover": [Color.from_hsv(float(hash(r.path) % 1000) / 1000.0, 0.5, 0.45), Color("#1c1d20")],
	}


## Ask the core what to ask next. The track in the player stays first; what you skipped goes last.
## `start`: the first one, asked of the last trained model so the app is ready at once; when
## that model is behind your labels it retrains behind the queue, then asks again.
func refresh_queue(count_new := false, start := false) -> void:
	var head: Dictionary = queue[0] if not queue.is_empty() else {}
	var skip: Array = _answered.keys()
	for t in _skipped:
		skip.append(t.path)
	if not head.is_empty():
		skip.append(head.path)
	skip.append_array(_front.keys())
	var before := {}
	for t in queue:
		before[t.path] = true
	_call(_model if start else _main, "queue", {"threshold": settings.threshold, "n": 200, "skip": skip, "stale_ok": start},
		func(q: Variant) -> void:
			if q == null:
				return
			if not _library_ready:
				_pending_queue = _apply_queue.bind(q, before, count_new)
			else:
				_apply_queue(q, before, count_new), "Choosing what to ask…" if not loaded else "")


func _apply_queue(q: Dictionary, before: Dictionary, count_new: bool) -> void:
	labels = int(q.labels)
	measured = int(q.measured)
	pool = int(q.pool)
	_acc = Vector2(q.accuracy[0], q.accuracy[1]) * 100.0
	var now_head: Dictionary = queue[0] if not queue.is_empty() else {}
	var out: Array[Dictionary] = []
	if not now_head.is_empty():
		out.append(now_head)
	for t in queue:
		if _front.has(t.path) and t != now_head and not _answered.has(t.path):
			out.append(t)
	var fresh := 0
	for it: Dictionary in q.items:
		var t: Dictionary = by_path.get(it.path, {})
		if t.is_empty() or t == now_head or _answered.has(t.path) or _front.has(t.path):
			continue
		t.tag = it.tag
		t.guess = _ui_of.get(int(it.guess), -1)
		t.guess_stars = int(it.stars)
		out.append(t)
		if it.tag in ["conflict", "old label"] and not before.has(t.path):
			fresh += 1
	for t in _skipped:
		if t != now_head and not _answered.has(t.path):
			out.append(t)
	queue = out
	if count_new:
		queued_by_sync = fresh
	var first := not loaded
	loaded = true
	if first:
		# only now is it known whether it is still calibrating
		_guess(reviewed)
	if q.get("stale", false):
		_call(_model, "train", {}, func(r: Variant) -> void:
			if r != null:
				refresh_queue()
				_measure_ahead())
	changed.emit()


# --- deciding --------------------------------------------------------------------------

## Your label for a track, from the player: it joins the reviewed table, newest first.
## energy and key only when you corrected them (0 and "" otherwise).
func decide(t: Dictionary, colour: int, rating: int, from: String, energy := 0, key := "") -> void:
	_call(_main, "decide", {"path": t.path, "colour": PALETTE.find(UI.COLOURS[colour].name) + 1, "stars": rating,
		"energy": energy, "key": key}, func(_r: Variant) -> void: pass)
	if energy > 0:
		t.energy = energy
	if key != "":
		t.key = key
	var mine: bool = t.source == "user"
	t.colour = colour
	if rating > 0:
		t.rating = rating
	t.source = "user"
	t.trust = 4 if t.plays > 0 else 3
	t.batch = true
	t.labelled_ts = int(Time.get_unix_time_from_system())
	t.labelled = Time.get_date_string_from_system()
	for r in reviewed:
		if r.flag == "just reviewed":
			r.flag = ""
	t.flag = "from library" if from == "library" else "just reviewed"
	if reviewed.has(t):
		reviewed.erase(t)
	elif int(t.get("joined", 0)) == 0:
		t.joined = int(Time.get_unix_time_from_system() * 1000)
	reviewed.push_front(t)
	t.erase("dropped")
	t.erase("fresh")
	if not mine:
		labels += 1
	_answered[t.path] = true
	queue.erase(t)
	_skipped.erase(t)
	_front.erase(t.path)
	if not t.embedded:
		_listen_to([t])
	_decided += 1
	if _decided % 10 == 0 or queue.size() < 20:
		refresh_queue()
	changed.emit()


## The track in the player goes to the end of the queue, asked again later.
func skip(t: Dictionary) -> void:
	queue.erase(t)
	_front.erase(t.path)
	if not _skipped.has(t):
		_skipped.append(t)
	queue.append(t)
	changed.emit()


## From the library: asked again, blind, right after the track in the player.
func add_to_queue(ts: Array) -> void:
	_to_front(ts, 1, "re-ask")


## Files dropped on the app, from rekordbox or Finder. On the queue: to its front in the order
## dropped, the first straight into the player. On the table: rows there, in the next Export as
## they are; one with no colour gets the model's guess. Returns {tracks, missing}: missing are
## the names that aren't in the collection.
func add_dropped(paths: PackedStringArray, to_table: bool) -> Dictionary:
	var out: String = ClassDB.class_call_static("Vibecheck", "call", "dropped",
		JSON.stringify({"root": settings.music, "xml": settings.xml, "paths": Array(paths)}))
	var r: Variant = JSON.parse_string(out)
	if not r is Dictionary or r.has("error"):
		failed.emit(str(r.error) if r is Dictionary else "reading the dropped files failed")
		return {"tracks": [], "missing": PackedStringArray()}
	var ts: Array = []
	var missing := PackedStringArray()
	for i in paths.size():
		if paths[i] == "":
			continue
		var t: Dictionary = by_path.get(r.ok[i], {}) if r.ok[i] != null else {}
		if t.is_empty():
			missing.append(paths[i].get_file())
		else:
			ts.append(t)
	if ts.is_empty():
		pass
	elif to_table:
		var at := int(Time.get_unix_time_from_system() * 1000)
		for i in ts.size():
			var t: Dictionary = ts[i]
			# stepped through in the player until confirmed; marked in the list until opened
			t.dropped = true
			t.fresh = true
			t.batch = true
			if not reviewed.has(t):
				t.joined = at + i
				reviewed.push_front(t)
		_call(_main, "add_to_batch", {"paths": ts.map(func(t: Dictionary) -> String: return t.path)}, func(_r: Variant) -> void: pass)
		_guess(ts)
		changed.emit()
	else:
		_to_front(ts, 0, "from rekordbox")
	return {"tracks": ts, "missing": missing}


## Rows in the list with no colour (dropped there) get the model's guess for colour and stars,
## saved as its label (guessed). Not while calibrating, nor with the guess turned off. Tracks it
## hasn't listened to are listened to first, a few seconds each.
func _guess(ts: Array) -> void:
	if calibrating() or not settings.show_guess:
		return
	var paths: Array = ts.filter(func(t: Dictionary) -> bool: return t.batch and t.colour < 0) \
		.map(func(t: Dictionary) -> String: return t.path)
	if paths.is_empty():
		return
	for p: String in paths:
		by_path[p].guessing = true
	guessing += paths.size()
	_call(_ask, "guess", {"paths": paths}, func(gs: Variant) -> void:
		guessing -= paths.size()
		_rest()
		for p: String in paths:
			by_path.get(p, {}).erase("guessing")
		if gs == null:
			changed.emit()
			return
		for g: Dictionary in gs:
			var t: Dictionary = by_path.get(g.path, {})
			var c: int = _ui_of.get(int(g.colour), -1)
			if t.is_empty() or t.colour >= 0 or c < 0:
				continue
			t.colour = c
			t.rating = maxi(int(g.stars), 0)
			t.source = "model"
			t.trust = 1
			t.batch = true
			t.embedded = true
			t.labelled_ts = int(Time.get_unix_time_from_system())
			t.labelled = Time.get_date_string_from_system()
			if not reviewed.has(t):
				reviewed.push_front(t)
		changed.emit(), "Guessing %d dropped track%s…" % [paths.size(), "" if paths.size() == 1 else "s"])


func _to_front(ts: Array, at: int, tag: String) -> void:
	at = mini(at, queue.size())
	var add: Array[Dictionary] = []
	for t: Dictionary in ts:
		if add.has(t):
			continue
		var i := queue.find(t)
		if i >= 0:
			queue.remove_at(i)
			if i < at:
				at -= 1
		_skipped.erase(t)
		_answered.erase(t.path)
		_front[t.path] = true
		t.tag = tag
		add.append(t)
	for j in add.size():
		queue.insert(at + j, add[j])
	changed.emit()


# --- export and sync -------------------------------------------------------------------

func export(done: Callable) -> void:
	var cut := int(Time.get_unix_time_from_system())
	_exporting = true
	_call(_main, "export", {}, func(e: Variant) -> void:
		_exporting = false
		if e != null:
			_seen_mtime = int(e.mtime)
			last_export = e
			for t in reviewed.duplicate():
				if t.labelled_ts <= cut:
					t.batch = false
					t.flag = ""
					reviewed.erase(t)
			changed.emit()
		done.call(e), "Writing key, energy and cue points into rekordbox.xml…")


func sync(done: Callable) -> void:
	_call(_main, "sync", {}, func(s: Variant) -> void:
		if s != null:
			last_sync = s
			_load_library(refresh_queue.bind(true))
		done.call(s), "Reading rekordbox.xml…")


## The history page: every sync and export, newest first.
func load_history(done: Callable) -> void:
	_call(_main, "history", {}, func(h: Variant) -> void:
		if h != null:
			done.call(h))


## One sync's or export's tracks, for its list.
func load_batch(id: int, done: Callable) -> void:
	_call(_main, "batch", {"id": id}, func(ts: Variant) -> void:
		if ts != null:
			done.call(ts))


## Called every few seconds: a new export from rekordbox starts a sync.
func poll() -> void:
	if not loaded or _exporting:
		return
	var m := FileAccess.get_modified_time(_abs(settings.xml))
	if m == 0 or m == _seen_mtime:
		return
	_seen_mtime = m
	exported_at = m
	_save("rekordbox", settings.xml, m)
	rekordbox_exported.emit()


# --- one track -------------------------------------------------------------------------

## The player's track: `on_wave` gets {duration, peaks}, then `on_parts` {sections, key,
## energy, cues, own_cues}; null on failure. Those of `ahead`, the tracks it may load next,
## are read meanwhile, so theirs are there when they come; calls for any other track that
## have not started yet are dropped, so stepping through fast doesn't wait for each one.
func wave(t: Dictionary, on_wave: Callable, on_parts: Callable, ahead: Array = []) -> void:
	_wv_now = t.path
	_wv_cb = {"wave": on_wave, "parts": on_parts}
	var want: Array[String] = [t.path]
	for a: Dictionary in ahead:
		if not a.is_empty() and a.path not in want:
			want.append(a.path)
	for p: String in _wv.keys():
		if p not in want:
			_wv.erase(p)
	_waves.drop(func(path: String) -> bool: return path in want)
	for p in want:
		_wv_load(p)
	for k: String in ["wave", "parts"]:
		if _wv[t.path].has(k):
			_wv_cb[k].call(_wv[t.path][k])


func _wv_load(path: String) -> void:
	if _wv.has(path):
		return
	var got := {}
	_wv[path] = got
	for k: String in ["wave", "parts"]:
		_call(_waves, k, {"path": path, "bins": 600}, func(r: Variant) -> void:
			if not is_same(_wv.get(path), got):
				return  # dropped while it was read
			got[k] = r
			if path == _wv_now:
				_wv_cb[k].call(r))


func audio_path(t: Dictionary) -> String:
	return _abs(settings.music).path_join(t.path)


## The cover in the file's tags, or null; read on the main thread, it is a few kilobytes.
## `px` square; only the small ones for the queue are kept.
func cover(t: Dictionary, px := 96) -> Texture2D:
	if px == 96 and _covers.has(t.path):
		return _covers[t.path]
	var b: PackedByteArray = ClassDB.class_call_static("Vibecheck", "cover", settings.music, t.path)
	var tex: Texture2D = null
	if b.size() > 4:
		var img := Image.new()
		var err := img.load_jpg_from_buffer(b) if b[0] == 0xFF else img.load_png_from_buffer(b)
		if err == OK:
			img.resize(px, px, Image.INTERPOLATE_BILINEAR)
			tex = ImageTexture.create_from_image(img)
	if px != 96:
		return tex
	if _covers.size() > 400:
		_covers.clear()
	_covers[t.path] = tex
	return tex


## The stats from the cache, at once; then, when your labels changed since the learning curve
## was measured, again with it measured (seconds). `done` gets each.
func load_stats(done: Callable) -> void:
	_call(_stats, "stats", {}, func(s: Variant) -> void:
		if s == null:
			return
		stats = s
		done.call(s)
		if not s.fresh:
			_call(_stats, "curve", {}, func(c: Variant) -> void:
				if c != null:
					stats = c
					done.call(c), "Measuring the model…"))


## After retraining, measure the learning curve too, so the stats page has it when opened.
func _measure_ahead() -> void:
	if _stats.busy == 0:
		_call(_stats, "curve", {}, func(c: Variant) -> void:
			if c != null:
				stats = c)


# --- listening in the background -------------------------------------------------------

## Tracks without a vector can't be learned from or asked: listen to them, a few at a time.
func _listen() -> void:
	if not settings.listen:
		_hearing = false
		_rest()
		return
	if _bg.busy > 0:
		return
	_hearing = true
	_call(_bg, "unheard", {"n": 4}, func(paths: Variant) -> void:
		if paths == null or paths.is_empty():
			listening = ""
			_hearing = false
			_rest()
			if paths != null:
				_say_listening(true)
			return
		var ts: Array[Dictionary] = []
		for p: String in paths:
			if by_path.has(p):
				ts.append(by_path[p])
		_listen_to(ts, _listen))


func _listen_to(ts: Array[Dictionary], then := Callable()) -> void:
	if ts.is_empty():
		return
	listening = ts[0].title
	var paths := ts.map(func(t: Dictionary) -> String: return t.path)
	var t0 := Time.get_ticks_msec()
	_call(_bg, "embed", {"paths": paths}, func(n: Variant) -> void:
		listening = ""
		if n == null:
			settings.listen = false
			_hearing = false
			_rest()
			return
		for t in ts:
			if not t.embedded:
				t.embedded = true
				embedded += 1
				pool += 1
		if then.is_valid():
			# timed in the background chain only: a single track may wait behind a batch
			var per := (Time.get_ticks_msec() - t0) / 1000.0 / ts.size()
			_per_track = per if _per_track == 0.0 else lerpf(_per_track, per, 0.3)
			_say_listening()
		changed.emit()
		if then.is_valid():
			then.call()
		else:
			_rest())


## Nothing left to hear or guess: the listening model's memory (over 1 GB) goes back to the
## system, until a track needs hearing again (3 s to load).
func _rest() -> void:
	if not _hearing and guessing == 0:
		_call(_bg, "rest", {}, func(_r: Variant) -> void: pass)


## Background listening as one line: what, the fraction heard, then "8% · 1,236 / 15,002 ·
## ~19 h left". Shown in the model strip, and in the console by `_say_listening`.
func listening_status() -> Array:
	var total := library.size()
	var heard := mini(embedded, total)
	if heard >= total:
		return ["all %s heard" % UI.thousands(total), 1.0, ""]
	var f := float(heard) / maxi(total, 1)
	var rest := "%d%% · %s / %s" % [int(f * 100), UI.thousands(heard), UI.thousands(total)]
	if not settings.listen:
		return ["listening off", f, rest]
	if _per_track > 0:
		var s := (total - heard) * _per_track
		rest += " · ~%s left" % ("%d h" % roundi(s / 3600) if s >= 7200 else "%d min" % maxi(roundi(s / 60), 1))
	return ["listening", f, rest]


## The listening line in the console, at most once a minute unless `now`.
func _say_listening(now := false) -> void:
	var at := Time.get_ticks_msec()
	if not now and _said_at >= 0 and at - _said_at < 60000:
		return
	_said_at = at
	var s := listening_status()
	var n := int(s[1] * 20)
	var bar: String = " [%s%s] " % ["#".repeat(n), ".".repeat(20 - n)] if s[1] < 1.0 else ""
	print("%s %s%s%s" % [Time.get_time_string_from_system(), s[0], bar, s[2]])


# --- settings ----------------------------------------------------------------------------

func set_setting(k: String, v: Variant) -> void:
	settings[k] = v
	if CORE_KEYS.has(k):
		_wv.clear()  # the cues shown follow the settings
		var value: Variant = v
		if k == "cues_lead":
			value = [0] if v == 0 else [16, 8]
		_call(_main, "set", {"section": CORE_KEYS[k][0], "key": CORE_KEYS[k][1], "value": value},
			func(_c: Variant) -> void: pass)
	else:
		_save("app", k, v)
	if k == "music" or k == "xml":
		queue.clear()
		_answered.clear()
		_skipped.clear()
		load_all()
	elif k == "threshold":
		refresh_queue()
	elif k == "listen" and v:
		_listen()
	changed.emit()


func _save(section: String, k: String, v: Variant) -> void:
	if not _saving:
		return
	var f := ConfigFile.new()
	f.load(CFG)
	f.set_value(section, k, v)
	f.save(CFG)


## One worker thread and its queue of calls into the core, answered on the main thread in order.
class Lane:
	var busy := 0
	var _thread := Thread.new()
	var _mutex := Mutex.new()
	var _sem := Semaphore.new()
	var _jobs: Array = []
	var _quit := false

	## `key` names what the call is for, for `drop`.
	func push(cmd: String, args: String, key: String, done: Callable) -> void:
		if not _thread.is_started():
			_thread.start(_work)
		_mutex.lock()
		_jobs.append([cmd, args, done, key])
		_mutex.unlock()
		busy += 1
		_sem.post()

	func _work() -> void:
		while true:
			_sem.wait()
			_mutex.lock()
			if _quit:
				_mutex.unlock()
				return
			if _jobs.is_empty():
				_mutex.unlock()
				continue  # dropped
			var j: Array = _jobs.pop_front()
			_mutex.unlock()
			var out: String = ClassDB.class_call_static("Vibecheck", "call", j[0], j[1])
			_mutex.lock()
			var quit := _quit
			_mutex.unlock()
			if quit:
				return  # closing: an answer queued now would never run, and keep everything alive
			_done.call_deferred(j[2], JSON.parse_string(out))

	func _done(done: Callable, r: Variant) -> void:
		busy -= 1
		done.call(r)

	## Forget the calls not started yet whose key `keep` turns down; they never answer.
	func drop(keep: Callable) -> void:
		_mutex.lock()
		var left := _jobs.filter(func(j: Array) -> bool: return keep.call(j[3]))
		busy -= _jobs.size() - left.size()
		_jobs = left
		_mutex.unlock()

	func stop() -> void:
		if not _thread.is_started():
			return
		_mutex.lock()
		_quit = true
		_jobs.clear()
		_mutex.unlock()
		_sem.post()
		_thread.wait_to_finish()
