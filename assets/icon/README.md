# Icon assets

Every file here is either loaded by code/CI or kept as a design master. Remove
derived variants that nothing loads.

| File | Consumer |
| --- | --- |
| `capin_icon.ico` | `crates/app/build.rs` (embedded into `capin.exe`), `.github/installer/Capin.iss` |
| `capin_icon.icns` | `.github/workflows/package.yml` (`Capin.app/Contents/Resources/AppIcon.icns`) |
| `capin_icon_1024.png` | `README.md` header |
| `tray/capin_tray_light.png` | `crates/app/src/tray.rs`, non-macOS light surface |
| `tray/capin_tray_dark.png` | `crates/app/src/tray.rs`, non-macOS dark surface |
| `tray/capin_tray_light_macos.png` | `crates/app/src/tray.rs`, macOS template image tinted by AppKit |
| `capin_icon.svg` | design master, not referenced by code or CI |
| `tray/capin_tray.svg` | design master, not referenced by code or CI |

Windows selects the light or dark tray PNG at runtime from the taskbar surface;
macOS uses a single template PNG so AppKit matches the menu bar appearance.
