//! Parity: apply the prototype's entries with the Rust writer.
//! usage: xml_check <src.xml> <out.xml> <entries.json>
use vibecheck::{palette, rekordbox, store::Label};

fn main() -> vibecheck::Res<()> {
    let a: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(std::env::var("HOME")?).join("Music/Collection");
    let js: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(&a[3])?)?;
    let entries: Vec<rekordbox::Entry> = js
        .iter()
        .map(|e| rekordbox::Entry {
            rel: e["rel"].as_str().unwrap().into(),
            level: match e["level"].as_str().unwrap() {
                "colour" => "colour",
                "hue" => "hue",
                "tone" => "tone",
                _ => "none",
            },
            label: Some(Label {
                colour: e["colour"].as_str().and_then(palette::by_name),
                stars: e["stars"].as_u64().map(|s| s as u8),
            }),
            key: e["key"].as_str().map(String::from),
            energy: e["energy"].as_u64().map(|v| v as u8),
            cues: e["cues"].as_array().map(|cs| {
                cs.iter()
                    .map(|c| {
                        c.as_array()
                            .unwrap()
                            .iter()
                            .map(|kv| (kv[0].as_str().unwrap().to_string(), kv[1].as_str().unwrap().to_string()))
                            .collect()
                    })
                    .collect()
            }),
        })
        .collect();
    let o = rekordbox::WriteOpts { name: "vibecheck-parity", tonality: true, comment: true, changed_only: false };
    let st = rekordbox::write(std::path::Path::new(&a[1]), std::path::Path::new(&a[2]), &root, &entries, &o)?;
    println!("{st:?}");
    Ok(())
}
