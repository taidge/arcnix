# Bundled terminal assets

These files are embedded into the desktop executable with `include_str!`; no CDN,
Node runtime or package manager is required at runtime or for normal Cargo builds.

| Package | Version | Files | License |
| --- | --- | --- | --- |
| `@xterm/xterm` | 6.0.0 | `xterm/xterm.js`, `xterm/xterm.css` | `xterm/LICENSE` (MIT) |
| `@xterm/addon-fit` | 0.11.0 | `addon-fit/addon-fit.js` | `addon-fit/LICENSE` (MIT) |

Official npm tarball URLs and verified SHA-512 integrity values are recorded in
`manifest.json`. Only the distributed JavaScript, CSS and license files were
extracted; source-map comments were removed because source maps are not bundled.

Upstream: https://github.com/xtermjs/xterm.js
