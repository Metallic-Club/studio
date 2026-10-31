//! Builds the Parkour template's scenes: `templates/parkour/main.mscene` and
//! `templates/parkour/tests/smoke.mscene`.
//!
//! The level is written as code so its numbers (gap widths, heights, timings) are easy to review
//! and tune; the scene files it writes are what Studio and the player load. Run it after changing
//! the level:
//!
//! ```text
//! cargo run -p metallic_runtime --example build_parkour
//! ```
//!
//! Character reach, for tuning: walking 16 studs/s and jumping 50 studs/s gives 0.51 s in the air
//! and about 8 studs of horizontal travel on the flat (12 when running); the jump apex is 6.4
//! studs. Gaps here are 3 to 7 studs edge to edge and steps up at most 3 studs (the tower; 2 elsewhere).
//! Every walkable top is a whole stud, so 1-stud slabs have their faces on the grid (spec 2.2).
mod level;
mod scripts;

use metallic_core::{Value, World, save_scene};
use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour");
    let world = level::build()?;
    std::fs::create_dir_all(root.join("tests"))?;
    save_scene(&world, &root.join("main.mscene"))?;
    let smoke = add_smoke_test(world)?;
    save_scene(&smoke, &root.join("tests/smoke.mscene"))?;
    println!("wrote {}", root.display());
    Ok(())
}

/// The main scene plus the smoke test's scripts and remotes.
fn add_smoke_test(mut w: World) -> Result<World, Box<dyn Error>> {
    let storage = w.root("ReplicatedStorage").ok_or("no ReplicatedStorage")?;
    for name in ["SmokeQuery", "SmokeReport"] {
        let r = w.create("RemoteEvent", Some(storage))?;
        w.set(r, "Name", Value::String(name.into()))?;
    }
    let sss = w
        .root("ServerScriptService")
        .ok_or("no ServerScriptService")?;
    let s = w.create("Script", Some(sss))?;
    w.set(s, "Name", Value::String("SmokeTest".into()))?;
    w.set(s, "Source", Value::String(scripts::SMOKE_SERVER.into()))?;
    let starter = w.root("StarterPlayer").ok_or("no StarterPlayer")?;
    let sps = w
        .find_first_child(starter, "StarterPlayerScripts")
        .ok_or("no StarterPlayerScripts")?;
    let c = w.create("LocalScript", Some(sps))?;
    w.set(c, "Name", Value::String("SmokeCounter".into()))?;
    w.set(c, "Source", Value::String(scripts::SMOKE_CLIENT.into()))?;
    Ok(w)
}
