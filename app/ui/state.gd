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
var last_export := {}
var last_sync := {}
var stats := {}
var listening := ""
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
var _answered := {}
var _skipped: Array[Dictionary] = []
var _covers := {}
var _seen_mtime := 0
var _exporting := false
var _decided := 0
var _saving := true
var _ui_of := {}


func _init() -> void:
	for i in UI.COLOURS.size():
		_ui_of[PALETTE.find(UI.COLOURS[i].name) + 1] = i
	var f := ConfigFile.new()
	if f.load(CFG) == OK:
		for k: String in f.get_section_keys("app"):
			settings[k] = f.get_value("app", k)
	elif DirAccess.dir_exists_absolute(_abs("~/Music/Collection/.vibecheck")):
		settings.music = "~/Music/Collection"
	# --music= and --xml= try other files without touching the saved settings
	for a in OS.get_cmdline_user_args():
		for k in ["music", "xml"]:
			if a.begins_with("--%s=" % k):
				settings[k] = a.split("=", true, 1)[1]
				_saving = false


## Close down: long work in the core gives up at its next safe point, then the
## workers are joined and what holds textures let go, so nothing leaks at exit.
func stop() -> void:
	ClassDB.class_call_static("Vibecheck", "stop")
	for l: Lane in [_main, _waves, _stats, _bg]:
		l.stop()
	_covers.clear()
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
	lane.push(cmd, JSON.stringify(args), func(r: Variant) -> void:
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
		_load_library(refresh_queue), "Reading your collection…")
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
		"labelled_ts": ts, "batch": r.batch, "embedded": r.embedded, "length": maxf(float(r.length), 1.0),
		"tag": "", "flag": "", "guess": -1, "guess_stars": -1,
		"cover": [Color.from_hsv(float(hash(r.path) % 1000) / 1000.0, 0.5, 0.45), Color("#1c1d20")],
	}


## Ask the core what to ask next. The track in the player stays first; what you skipped goes last.
func refresh_queue(count_new := false) -> void:
	var head: Dictionary = queue[0] if not queue.is_empty() else {}
	var skip: Array = _answered.keys()
	for t in _skipped:
		skip.append(t.path)
	if not head.is_empty():
		skip.append(head.path)
	var before := {}
	for t in queue:
		before[t.path] = true
	_call(_main, "queue", {"threshold": settings.threshold, "n": 200, "skip": skip}, func(q: Variant) -> void:
		if q == null:
			return
		labels = int(q.labels)
		measured = int(q.measured)
		pool = int(q.pool)
		_acc = Vector2(q.accuracy[0], q.accuracy[1]) * 100.0
		var now_head: Dictionary = queue[0] if not queue.is_empty() else {}
		var out: Array[Dictionary] = []
		if not now_head.is_empty():
			out.append(now_head)
		var fresh := 0
		for it: Dictionary in q.items:
			var t: Dictionary = by_path.get(it.path, {})
			if t.is_empty() or t == now_head or _answered.has(t.path):
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
		loaded = true
		changed.emit(), "Choosing what to ask…" if not loaded else "")


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
	reviewed.push_front(t)
	if not mine:
		labels += 1
	_answered[t.path] = true
	queue.erase(t)
	_skipped.erase(t)
	if not t.embedded:
		_listen_to([t])
	_decided += 1
	if _decided % 10 == 0 or queue.size() < 20:
		refresh_queue()
	changed.emit()


## The track in the player goes to the end of the queue, asked again later.
func skip(t: Dictionary) -> void:
	queue.erase(t)
	if not _skipped.has(t):
		_skipped.append(t)
	queue.append(t)
	changed.emit()


func add_to_queue(ts: Array) -> void:
	var at := mini(1, queue.size())
	for t: Dictionary in ts:
		if queue.has(t):
			continue
		t.tag = "re-ask"
		_answered.erase(t.path)
		queue.insert(at, t)
		at += 1
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

## The waveform and parts of a track; `done` gets {duration, peaks, sections} or null.
func wave(t: Dictionary, done: Callable) -> void:
	_call(_waves, "wave", {"path": t.path, "bins": 600}, done)


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


func load_stats(done: Callable) -> void:
	_call(_stats, "stats", {}, func(s: Variant) -> void:
		if s != null:
			stats = s
		done.call(s), "Measuring the model…")


# --- listening in the background -------------------------------------------------------

## Tracks without a vector can't be learned from or asked: listen to them, a few at a time.
func _listen() -> void:
	if not settings.listen or _bg.busy > 0:
		return
	_call(_bg, "unheard", {"n": 4}, func(paths: Variant) -> void:
		if paths == null or paths.is_empty():
			listening = ""
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
	_call(_bg, "embed", {"paths": paths}, func(n: Variant) -> void:
		listening = ""
		if n == null:
			settings.listen = false
			return
		for t in ts:
			if not t.embedded:
				t.embedded = true
				embedded += 1
				pool += 1
		changed.emit()
		if then.is_valid():
			then.call())


# --- settings ----------------------------------------------------------------------------

func set_setting(k: String, v: Variant) -> void:
	settings[k] = v
	if CORE_KEYS.has(k):
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

	func push(cmd: String, args: String, done: Callable) -> void:
		if not _thread.is_started():
			_thread.start(_work)
		_mutex.lock()
		_jobs.append([cmd, args, done])
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

	func stop() -> void:
		if not _thread.is_started():
			return
		_mutex.lock()
		_quit = true
		_jobs.clear()
		_mutex.unlock()
		_sem.post()
		_thread.wait_to_finish()
