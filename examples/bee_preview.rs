//! Print the bee mascot sprite through the real render pipeline. `cargo run --example bee_preview`.
// Aliased: the crate and the mascot module are both called `bee` now that the harness library and
// the application are one package, and an unaliased import would shadow the crate root.
use bee::viz::bee as mascot;
use bee::viz::sprite_render::{render_frame_with, ColorMode};

fn main() {
    for line in render_frame_with(&mascot::sprite(), ColorMode::TrueColor, true) {
        println!("  {line}");
    }
}
