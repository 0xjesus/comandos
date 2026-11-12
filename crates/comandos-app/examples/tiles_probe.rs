//! Feeds raw tmux client output to the terminal and prints the panes the
//! tile scanner finds: `cargo run --example tiles_probe -- out.raw 176 22`.
use comandos_app::term::{
    engine::{Palette, TermEngine},
    paint::{CellGeom, FrameCache},
};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("raw file");
    let cols: u16 = args[2].parse().unwrap_or(176);
    let rows: u16 = args[3].parse().unwrap_or(22);
    let palette = Palette::xterm_default([211, 218, 228], [10, 13, 19], [211, 218, 228], [10, 13, 19], [211, 218, 228]);
    let epoch = std::time::Instant::now();
    let mut term = TermEngine::new(cols, rows, 0, palette, false, epoch);
    term.feed(&bytes, std::time::Instant::now());
    let later = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let _ = term.tick(later);
    let geom = CellGeom { cell_w: 9., cell_h: 20., origin_x: 4., origin_y: 4., dpr: 1., font_size: 11. };
    let mut frame = FrameCache::default();
    println!("refreshed={}", frame.refresh(&term, &geom));
    println!("{}", frame.border_debug().replace(" r", "\nr"));
    println!("{:?}", frame.tiles(rows - 1));
}
