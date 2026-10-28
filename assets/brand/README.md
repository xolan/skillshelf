# Brand files

Each SVG comes in a dark version, made for dark backgrounds, and a light one (`-light`). The app
icons use the dark versions on every background, since the app can't tell what the desktop
behind them looks like; on light taskbars and panels their grey handle is faint.

| File | What it is | Where it's used |
| --- | --- | --- |
| `xolkit.svg`, `xolkit-light.svg` | The case on its rounded tile, for 64 px and up | The scalable Linux icon and the large `.ico` sizes use the dark one. The light one isn't used yet. |
| `xolkit-glyph.svg`, `xolkit-glyph-light.svg` | The case alone, for 32–48 px | The 32 and 48 px Linux icons and `.ico` sizes use the dark one. The light one isn't used yet. |
| `xolkit-small.svg`, `xolkit-small-light.svg` | The case alone, drawn at 16 px, for 16–24 px | The title bar uses the one matching the theme. The 16–24 px Linux icons and `.ico` sizes use the dark one. |
| `xolkit-wordmark.svg`, `xolkit-wordmark-light.svg` | The case and the name, the letters outlined from Geist SemiBold | The README, by the reader's color scheme |
| `xolkit.ico` | The Windows icon, made from the dark SVGs by `mise run icon` | The Windows executable |
