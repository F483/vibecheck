//! Godot's view of the core: thin classes that call into `vibecheck`, nothing else.

use godot::prelude::*;

struct Extension;

#[gdextension]
unsafe impl ExtensionLibrary for Extension {}

#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct Vibecheck {}

#[godot_api]
impl Vibecheck {
    #[func]
    fn version() -> GString {
        vibecheck::VERSION.into()
    }
}
