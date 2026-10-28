//! Renders the icons in `assets/brand` into `assets/brand/xolkit.ico`, the icon Windows shows for
//! the app. Run it through `mise run icon` after changing them.

use std::path::Path;

use anyhow::Context;
use resvg::{tiny_skia, usvg};

/// The sizes Windows picks from at 100–200% scaling: lists and title bars (16–40), the taskbar
/// and Start menu (32–64), and large Explorer views (256).
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() -> anyhow::Result<()> {
    let brand = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brand");
    let load = |file: &str| -> anyhow::Result<usvg::Tree> {
        let svg = std::fs::read(brand.join(file)).with_context(|| format!("reading {file}"))?;
        usvg::Tree::from_data(&svg, &usvg::Options::default())
            .with_context(|| format!("parsing {file}"))
    };
    let small = load("xolkit-small.svg")?;
    let glyph = load("xolkit-glyph.svg")?;
    let tile = load("xolkit.svg")?;
    let mut images = Vec::new();
    for size in SIZES {
        // The tile only on the large sizes; the case alone below, drawn at 16 px for 16–24.
        let tree = match size {
            ..=24 => &small,
            25..=48 => &glyph,
            _ => &tile,
        };
        let mut pixmap = tiny_skia::Pixmap::new(size, size).context("empty icon size")?;
        let scale = size as f32 / tree.size().width();
        let transform = tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(tree, transform, &mut pixmap.as_mut());
        // PNG keeps the large image small; everything reads bitmaps at the smaller sizes.
        let image = if size >= 256 {
            pixmap.encode_png()?
        } else {
            bitmap(&pixmap)
        };
        images.push((size, image));
    }
    std::fs::write(brand.join("xolkit.ico"), ico(&images)).context("writing xolkit.ico")?;
    Ok(())
}

/// A 32-bit bitmap as icons hold it: a BITMAPINFOHEADER of twice the height, for the image and
/// the 1-bit AND mask after it (unused with an alpha channel), rows bottom-up in BGRA.
fn bitmap(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
    let (width, height) = (pixmap.width(), pixmap.height());
    let mask_size = width.div_ceil(32) * 4 * height;
    let mut out = Vec::new();
    out.extend(40u32.to_le_bytes()); // header size
    out.extend(width.to_le_bytes());
    out.extend((2 * height).to_le_bytes());
    out.extend(1u16.to_le_bytes()); // color planes
    out.extend(32u16.to_le_bytes()); // bits per pixel
    out.extend(0u32.to_le_bytes()); // BI_RGB, uncompressed
    out.extend((width * height * 4 + mask_size).to_le_bytes());
    out.extend([0; 16]); // resolution and palette, unused
    for row in pixmap.pixels().chunks(width as usize).rev() {
        for pixel in row {
            let color = pixel.demultiply();
            out.extend([color.blue(), color.green(), color.red(), color.alpha()]);
        }
    }
    out.resize(out.len() + mask_size as usize, 0);
    out
}

/// An ICO file of the given images, each a bitmap or a PNG.
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes()); // reserved
    out.extend(1u16.to_le_bytes()); // type: icon
    out.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        // A side of 256 is written as 0.
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend([side, side, 0, 0]); // width, height, palette size, reserved
        out.extend(1u16.to_le_bytes()); // color planes
        out.extend(32u16.to_le_bytes()); // bits per pixel
        out.extend((png.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend(png);
    }
    out
}
