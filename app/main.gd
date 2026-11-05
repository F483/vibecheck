extends Control


func _ready() -> void:
	var text := "vibecheck core %s" % Vibecheck.version()
	$Label.text = text
	print(text)
