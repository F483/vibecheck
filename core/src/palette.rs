//! The palette, which is fixed: rekordbox's eight colour tags and six star
//! ratings. Hue and tone form a 4x2 grid whose cells are the colours, so a
//! confident hue plus a confident tone *is* a colour.

#[derive(Debug, PartialEq, Eq)]
pub struct Colour {
    pub id: i64,
    pub name: &'static str,
    pub hue: &'static str,
    pub tone: &'static str,
    /// as rekordbox writes it in its xml
    pub rgb: &'static str,
}

pub const COLOURS: [Colour; 8] = [
    Colour { id: 1, name: "Red", hue: "Warm", tone: "Dark", rgb: "0xFF0000" },
    Colour { id: 2, name: "Orange", hue: "Warm", tone: "Light", rgb: "0xFFA500" },
    Colour { id: 3, name: "Yellow", hue: "Acidic", tone: "Light", rgb: "0xFFFF00" },
    Colour { id: 4, name: "Green", hue: "Acidic", tone: "Dark", rgb: "0x00FF00" },
    Colour { id: 5, name: "Aqua", hue: "Cool", tone: "Light", rgb: "0x25FDE9" },
    Colour { id: 6, name: "Blue", hue: "Cool", tone: "Dark", rgb: "0x0000FF" },
    Colour { id: 7, name: "Purple", hue: "Vibrant", tone: "Dark", rgb: "0x660099" },
    Colour { id: 8, name: "Pink", hue: "Vibrant", tone: "Light", rgb: "0xFF007F" },
];

pub const HUES: [&str; 4] = ["Warm", "Acidic", "Cool", "Vibrant"];
pub const TONES: [&str; 2] = ["Dark", "Light"];
pub const MAX_STARS: u8 = 5;
/// rekordbox stores a rating as 0, 51, 102, 153, 204 or 255
pub const STAR_STEP: i64 = 51;

pub type C = &'static Colour;

pub fn by_id(id: i64) -> Option<C> {
    COLOURS.iter().find(|c| c.id == id)
}

pub fn by_name(name: &str) -> Option<C> {
    COLOURS.iter().find(|c| c.name == name)
}

pub fn by_rgb(rgb: &str) -> Option<C> {
    COLOURS.iter().find(|c| c.rgb == rgb)
}

/// Both axes, or nothing: one alone narrows the choice without making it.
pub fn from_grid(hue: Option<&str>, tone: Option<&str>) -> Option<C> {
    let (h, t) = (hue?, tone?);
    COLOURS.iter().find(|c| c.hue == h && c.tone == t)
}

pub fn stars_from_rating(raw: Option<&str>) -> u8 {
    let v: i64 = raw.and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    (v / STAR_STEP).clamp(0, MAX_STARS as i64) as u8
}

pub fn rating_from_stars(stars: u8) -> String {
    (stars as i64 * STAR_STEP).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_is_exact() {
        for h in HUES {
            for t in TONES {
                assert!(from_grid(Some(h), Some(t)).is_some());
            }
        }
        assert_eq!(from_grid(Some("Warm"), None), None);
    }

    #[test]
    fn ratings_round_trip() {
        for s in 0..=MAX_STARS {
            assert_eq!(stars_from_rating(Some(&rating_from_stars(s))), s);
        }
        assert_eq!(stars_from_rating(None), 0);
    }
}
