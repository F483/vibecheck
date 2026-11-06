//! How long loading the audio model takes.
fn main() -> vibecheck::Res<()> {
    let t = std::time::Instant::now();
    let _e = vibecheck::embed::Embedder::load()?;
    println!("rust CLAP load {:.2}", t.elapsed().as_secs_f64());
    Ok(())
}
