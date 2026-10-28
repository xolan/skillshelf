//! Default component icons plus the extra Lucide icons xolkit uses, and the embedded fonts.

use std::borrow::Cow;

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

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [Puzzle, Store, Trash, Download, Lock, Layers, Pencil]
);

/// The logo for the title bar, for dark and light themes: the case alone, drawn for 16 px.
pub const LOGO: &str = "brand/xolkit-small.svg";
pub const LOGO_LIGHT: &str = "brand/xolkit-small-light.svg";
const LOGO_SVG: &[u8] = include_bytes!("../../../assets/brand/xolkit-small.svg");
const LOGO_LIGHT_SVG: &[u8] = include_bytes!("../../../assets/brand/xolkit-small-light.svg");

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            LOGO => return Ok(Some(Cow::Borrowed(LOGO_SVG))),
            LOGO_LIGHT => return Ok(Some(Cow::Borrowed(LOGO_LIGHT_SVG))),
            _ => {}
        }
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
