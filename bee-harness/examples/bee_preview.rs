//! Print the bee mascot sprite through the real render pipeline. `cargo run -p bee-harness --example bee_preview`.
use bee_harness::viz::bee;
use bee_harness::viz::sprite_render::{render_frame_with, ColorMode};

fn main() {
    for line in render_frame_with(&bee::sprite(), ColorMode::TrueColor, true) {
        println!("  {line}");
    }
}
