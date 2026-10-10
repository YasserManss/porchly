# Asset attribution

Every non-code asset in this repository (images, icons, fonts, sample files) is listed here with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if a file under `assets/` is missing from this table, or if the table lists a file that doesn't exist.

**Policy:** every asset is original work by Porchly contributors, or third-party material under a licence that allows redistribution (OSI open source, public domain / CC0, or Creative Commons that allows redistribution). No vendor iconography, artwork, fonts or templates. Prefer art generated in code (the UI icon set will live in `crates/ui-egui/src/icons.rs`).

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/brand/porchly-icon.svg` | Porchly contributors | original | CC0-1.0 | The app icon: a porch lantern at night |
| `assets/brand/porchly-icon-256.png` | Porchly contributors | rendered from `porchly-icon.svg` | CC0-1.0 | Window icon |
| `assets/brand/porchly-icon-1024.png` | Porchly contributors | rendered from `porchly-icon.svg` | CC0-1.0 | macOS app icon (CI makes the .icns) |
| `assets/brand/porchly.ico` | Porchly contributors | rendered from `porchly-icon.svg` | CC0-1.0 | Windows program icon |
| `assets/brand/porchly-logo-light.svg` | Porchly contributors | original | CC0-1.0 | Logo for light backgrounds; text set in Noto Sans |
| `assets/brand/porchly-logo-dark.svg` | Porchly contributors | original | CC0-1.0 | Logo for dark backgrounds; text set in Noto Sans |
| `assets/brand/porchly-logo-light.png` | Porchly contributors | rendered from the SVG | CC0-1.0 (artwork); wordmark glyphs from Noto Sans, SIL OFL 1.1 | README logo |
| `assets/brand/porchly-logo-dark.png` | Porchly contributors | rendered from the SVG | CC0-1.0 (artwork); wordmark glyphs from Noto Sans, SIL OFL 1.1 | README logo, dark mode |
| `assets/screenshots/hero.png` | Porchly contributors | `snapshot` example (sample data) | CC0-1.0 | README |
| `assets/screenshots/import.png` | Porchly contributors | `snapshot` example (sample data) | CC0-1.0 | README |
| `assets/screenshots/porch-light.png` | Porchly contributors | `snapshot` example (sample data) | CC0-1.0 | README |
| `assets/screenshots/functions.png` | Porchly contributors | `snapshot` example (sample data) | CC0-1.0 | README |
| `assets/screenshots/dark.png` | Porchly contributors | `snapshot` example (sample data) | CC0-1.0 | README |

The name "Porchly" and the lantern logo identify this project. Use them to refer to Porchly;
please don't use them in a way that suggests your product is Porchly or made by its authors.
