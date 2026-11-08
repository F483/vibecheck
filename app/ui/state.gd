extends RefCounted
## App state, filled with made-up tracks until the Rust core is wired in.
## The shapes follow docs/ui/decisions.md; swap the mock for core calls later.

signal changed

const TITLES := [
	"Low Tide", "Undertow (Dub)", "Glasshouse", "Paper Moon", "Night Bus", "Salt Air", "Coral",
	"Pale Signal", "Glasswing", "Sunday Brass", "Deep Field", "Northbound", "Undertow",
	"Velvet Hour", "Hot Wire", "Brass Step", "Ember", "Flash", "Bright Day", "Shoreline", "Night Swim",
]
const WORD_A := ["Blue", "Late", "Soft", "Hollow", "Static", "Golden", "Silent", "Lunar", "Neon",
	"Quiet", "Wild", "Slow", "Electric", "Paper", "Glass", "Velvet"]
const WORD_B := ["Hour", "Signal", "Tide", "Garden", "Motion", "Room", "Weather", "Bloom", "Circuit",
	"Harbour", "Light", "Dust", "Fever", "Season", "Mirror", "Orbit"]
const ARTISTS := ["Harbour Lights", "Deep Field", "Pale Signal", "Mirrorball", "Shoreline", "Drift Line",
	"Northbound", "Undertow", "Night Swim", "Copper Sun", "Ember", "Brass Step", "Lemon Grove", "Salt Air"]
const MIXES := ["original mix", "extended mix", "dub", "club mix", "edit"]
const QUEUE_TAGS := ["calibration", "conflict", "calibration", "unsure", "calibration", "re-ask",
	"calibration", "old label"]

var library: Array[Dictionary] = []
var queue: Array[Dictionary] = []
var reviewed: Array[Dictionary] = []
var labels := 47
var library_total := 4812
var export_age_days := 9
var queued_by_sync := 7
var last_export := {}
var last_sync := {}
var settings := {
	"music": "~/Music/Collection",
	"xml": "~/Documents/rekordbox.xml",
	"threshold": 100,
	"show_guess": true,
	"write_tonality": true,
	"write_comment": true,
	"warn_days": 7,
	"cues_write": true,
	"cues_lead": 0, # 0: one cue at each change (A-D), 1: 16 + 8 bars before (A-H)
	"cues_loops": {}, # hot-cue slot -> loop length in bars
	"cues_memory": true,
}


func _init() -> void:
	var rng := RandomNumberGenerator.new()
	rng.seed = 7
	for i in 300:
		library.append(_track(rng, i))
	for i in 47:
		var t := library[40 + i]
		t.trust = 3
		reviewed.append(t)
	reviewed[0].flag = "just reviewed"
	for i in 53:
		var t := library[100 + i]
		t.tag = QUEUE_TAGS[i % QUEUE_TAGS.size()]
		queue.append(t)


func _track(rng: RandomNumberGenerator, i: int) -> Dictionary:
	var title: String = TITLES[i] if i < TITLES.size() else "%s %s" % [WORD_A[rng.randi() % WORD_A.size()], WORD_B[rng.randi() % WORD_B.size()]]
	var plays := 0 if rng.randf() < 0.35 else rng.randi_range(1, 31)
	var hue := rng.randf()
	return {
		"id": i,
		"title": title,
		"artist": ARTISTS[rng.randi() % ARTISTS.size()],
		"mix": MIXES[rng.randi() % MIXES.size()],
		"colour": rng.randi_range(0, 7),
		"energy": rng.randi_range(2, 9),
		"rating": rng.randi_range(2, 5),
		"bpm": rng.randi_range(118, 128),
		"key": "%d%s" % [rng.randi_range(1, 12), "A" if rng.randf() < 0.6 else "B"],
		"trust": 4 if plays > 0 else rng.randi_range(1, 3),
		"plays": plays,
		"last": "never" if plays == 0 else "2026-%02d-%02d" % [rng.randi_range(1, 10), rng.randi_range(1, 28)],
		"added": "20%02d-%02d" % [rng.randi_range(18, 26), rng.randi_range(1, 12)],
		"length": rng.randf_range(300, 480),
		"seed": rng.randi(),
		"cover": [Color.from_hsv(hue, 0.6, 0.5), Color.from_hsv(fmod(hue + 0.15, 1.0), 0.7, 0.15)],
		"tag": "",
		"flag": "",
	}


func is_ready() -> bool:
	return settings.music != "" and settings.xml != ""


func calibrating() -> bool:
	return labels < settings.threshold


## How often the model's colour is right, as a range that narrows with labels.
## Shaped after the measured curve: 56% at 100 labels, 65% at 300, 69% at 800.
func accuracy() -> Vector2i:
	var t := clampf(log(maxf(labels, 10) / 10.0) / log(100.0), 0, 1)
	var mid := lerpf(45, 68, t)
	var half := lerpf(18, 3, t)
	return Vector2i(roundi(mid - half), roundi(mid + half))


func export_old() -> bool:
	return export_age_days > settings.warn_days


## A decision from the player: the track joins the reviewed table, newest first.
func decide(t: Dictionary, colour: int, rating: int, from: String) -> void:
	var fresh := not reviewed.has(t)
	t.colour = colour
	if rating > 0:
		t.rating = rating
	t.trust = maxi(t.trust, 3)
	for r in reviewed:
		if r.flag == "just reviewed":
			r.flag = ""
	t.flag = "from library" if from == "library" else "just reviewed"
	if fresh:
		reviewed.push_front(t)
		labels += 1
	changed.emit()


func add_to_queue(ts: Array) -> void:
	for t: Dictionary in ts:
		if not queue.has(t):
			t.tag = "re-ask"
			queue.append(t)
	changed.emit()


func record_export() -> void:
	last_export = {"count": reviewed.size(), "name": "vibecheck-%s" % Time.get_date_string_from_system()}


## What a sync would find: played tracks move to level 4, corrections to 3.
func apply_sync() -> void:
	var played := 0
	var corrected := 0
	for i in reviewed.size():
		var t := reviewed[i]
		if i % 3 == 0:
			t.trust = 4
			t.plays += 1
			played += 1
		elif i % 7 == 0:
			corrected += 1
	var front: Array[Dictionary] = []
	for i in queued_by_sync:
		var t := library[200 + i]
		t.tag = ["conflict", "old label", "unsure"][i % 3]
		front.append(t)
	# after the track in the player, so syncing never swaps it out mid-listen
	queue = queue.slice(0, 1) + front + queue.slice(1)
	last_sync = {"played": played, "corrected": corrected, "untouched": reviewed.size() - played - corrected,
		"queued": queued_by_sync}
	export_age_days = 0
	changed.emit()
