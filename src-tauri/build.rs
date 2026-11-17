use std::{env, fs, path::PathBuf};

const DISPLAY_GLYPHS: &str = " ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.:+/-?";

fn build_font_atlas(
    font: &fontdue::Font,
    px: f32,
    cell_width: usize,
    cell_height: usize,
) -> Vec<u8> {
    let mut atlas = Vec::with_capacity(DISPLAY_GLYPHS.chars().count() * cell_width * cell_height);
    for character in DISPLAY_GLYPHS.chars() {
        let (metrics, bitmap) = font.rasterize(character, px);
        let mut cell = vec![0u8; cell_width * cell_height];
        let left = cell_width.saturating_sub(metrics.width) / 2;
        let top = cell_height.saturating_sub(metrics.height) / 2;
        let copy_width = metrics.width.min(cell_width);
        let copy_height = metrics.height.min(cell_height);
        for row in 0..copy_height {
            let source = row * metrics.width;
            let target = (top + row) * cell_width + left;
            cell[target..target + copy_width].copy_from_slice(&bitmap[source..source + copy_width]);
        }
        atlas.extend_from_slice(&cell);
    }
    atlas
}

fn main() {
    println!("cargo:rerun-if-env-changed=LYNX_PACKAGE_SIGNING_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=LYNX_LEARN_TRUSTED_KEYS_JSON");
    println!("cargo:rerun-if-changed=../res/font/SourceHanSansSC-Normal-Min.ttf");

    let font_path = PathBuf::from("../res/font/SourceHanSansSC-Normal-Min.ttf");
    let font_bytes = fs::read(&font_path).expect("read hardware monitor font");
    let font = fontdue::Font::from_bytes(font_bytes, fontdue::FontSettings::default())
        .expect("parse hardware monitor font");
    let mut atlas = build_font_atlas(&font, 13.0, 9, 14);
    atlas.extend_from_slice(&build_font_atlas(&font, 19.0, 16, 22));
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("hardware_monitor_font.bin");
    fs::write(output, atlas).expect("write hardware monitor font atlas");

    tauri_build::build()
}
