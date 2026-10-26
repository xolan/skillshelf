//! Default component icons plus the extra Lucide icons skillshelf uses, the embedded fonts and
//! the window icon.

use std::borrow::Cow;
use std::sync::Arc;

use gpui_kit::{AssetSource, Result, SharedString};

/// Geist and Geist Mono, under the SIL Open Font License (`fonts/OFL.txt`).
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../fonts/Geist-Regular.ttf"),
    include_bytes!("../fonts/Geist-Italic.ttf"),
    include_bytes!("../fonts/Geist-Medium.ttf"),
    include_bytes!("../fonts/Geist-SemiBold.ttf"),
    include_bytes!("../fonts/Geist-Bold.ttf"),
    include_bytes!("../fonts/GeistMono-Regular.ttf"),
    include_bytes!("../fonts/GeistMono-Bold.ttf"),
];

/// The app icon for the window (X11 shows it; Wayland takes it from the desktop entry).
pub fn window_icon() -> Option<Arc<image::RgbaImage>> {
    let png = include_bytes!("../../../assets/linux/skillshelf.png");
    let icon = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    Some(Arc::new(icon.into_rgba8()))
}

gpui_kit::assets::icon_assets!(ExtraIcons, [LibraryBig, Puzzle, Store, Trash, Download]);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
