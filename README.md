<p align="center"><img src="assets/icon/capin_icon_1024.png" width="128" alt="Capin logo"></p>

<a id="chinese"></a>

# Capin

Capin 是面向 Windows 10+ 与 macOS 13+ 的截图、标注、贴图、OCR、长截图与录屏工具。目标简单、高效、类原生。

中文 | [English](#english)

## 功能

- **窗口截图**：默认拖拽选区；按住 Alt 高亮并点击窗口截图，避免误截；支持固定比例与光标捕捉；桌面空白处单击截取整屏。
- **窗口质感**：窗口截图合成圆角、阴影、内边距与背景画布，支持透明、浅色、深色、渐变、壁纸背景与透明 PNG；可在设置中调整或关闭。
- **标注**：矩形、椭圆、箭头、荧光笔、文字、高斯模糊、序号、聚光灯、对象级橡皮、撤销重做；选中对象可改颜色与线宽，双击文字可再编辑。
- **贴图**：图片/文本/文件/颜色贴图，置顶、透明度、缩放、旋转、多贴、鼠标穿透、重启恢复、贴图标注与 OCR；图片贴图可按设置自动识别文字与二维码，文字可直接划选并自动复制。
- **OCR**：框选识别、复制文本、二维码。
- **长截图**：自动滚动（macOS 需辅助功能权限）+ 重叠匹配与固定顶栏裁除；手动滚动兜底。
- **录屏**：Windows 与 macOS 区域或窗口录制（Alt 点击窗口），不采集声音与摄像头；MP4（系统 H.264 编码）；暂停、计时、光标录入；录制条不入镜。
- **系统**：托盘、全局热键、剪贴板、自动保存、开机自启；截图默认保存到系统图片目录的 `Capin` 文件夹，录屏默认保存到系统视频目录的 `Capin` 文件夹，可在设置中修改；设置页跟随系统亮暗与强调色；界面中英文，默认中文；不上传，无遥测。

## 安装

从 [Releases](https://github.com/fynhq/capin/releases) 下载对应包：

| 平台 | 安装包 | 绿色包 |
|---|---|---|
| Windows x64 | `Capin_Windows_x64_vX.Y.Z_setup.exe` | `Capin_Windows_x64_vX.Y.Z.zip` |
| Windows arm64 | `Capin_Windows_arm64_vX.Y.Z_setup.exe` | `Capin_Windows_arm64_vX.Y.Z.zip` |
| macOS arm64 | `Capin_macOS_arm64_vX.Y.Z.dmg` | — |
| macOS x64 | `Capin_macOS_x64_vX.Y.Z.dmg` | — |

- Windows 安装包按当前用户安装，无需管理员权限；绿色包解压后直接运行 `Capin.exe`。
- macOS 打开 dmg，将 `Capin.app` 拖到 `/Applications`（或 `~/Applications`），启动应用而不是 `Contents/MacOS` 内的二进制。
- 如打开有拦截：Windows SmartScreen 选择「更多信息 → 仍要运行」；macOS 执行：

```sh
xattr -d -r com.apple.quarantine /Applications/Capin.app
```

- 升级：Windows 运行新安装包覆盖安装，或替换绿色包目录；macOS 替换应用。卸载：Windows 在「设置 → 应用」中卸载，或直接删除绿色包目录；macOS 删除应用。配置文件保留在用户目录，截图与录屏保存在图片/视频目录，按需手动删除。
- 每个 Release 附带 `SHA256SUMS`；下载后可在包目录执行 `sha256sum -c SHA256SUMS`（macOS 用 `shasum -a 256 -c SHA256SUMS`）。

## 使用

- 通过托盘菜单或全局热键开始截图；默认拖拽选择区域，按住 Alt 高亮并点击窗口截图，Esc 取消。
- 截图时按 `R` 切换自由/1:1/4:3/16:9 比例；放大镜显示光标下的颜色和选区尺寸，按 `Shift` 切换十六进制/RGB，按 `C` 复制当前格式的颜色值。
- 全局热键：`Win/Cmd+Shift+S` 截图，`Win/Cmd+Shift+D` 钉图；可在设置中修改。
- 编辑器内复制/保存使用系统快捷键 `Ctrl/Cmd+C`、`Ctrl/Cmd+S`。
- 托盘右键菜单提供截图、录屏、长截图、OCR、截图文件夹、设置、恢复贴图交互、重启与退出；关闭窗口后应用留在托盘。
- 截图后进入标注编辑器：绘制、文字、序号、聚光灯、模糊，复制、保存或贴图；选中已有对象可改颜色与线宽，双击文字可再次编辑。
- 窗口截图的圆角、阴影、留白与背景在「设置 → 截图 → 窗口截图」中调整，这些样式只作用于窗口截图；默认透明背景并输出带 Alpha 的 PNG。
- 贴图窗口可置顶、调整透明度与缩放，图片贴图可顺时针旋转 90°；默认不显示操作栏，左键移动或划选文字，右键菜单可旋转、编辑、复制、保存、关闭；也可开启鼠标穿透。开启穿透后从托盘选择「恢复贴图交互」恢复操作；贴图会在重启后恢复。
- 图片贴图可按「设置 → 截图 → 贴图」中的开关自动识别文字与二维码：在文字上拖动即可划选，松开后自动复制；双击选择单词；在非文字区域按住左键拖动可移动贴图。
- 录屏以系统 H.264 编码保存为 MP4；默认包含光标，可在「设置 → 录屏」中关闭；拖拽选区或按住 Alt 点击窗口开始录制，录制过程可暂停，停止后自动保存。
- 所有截图与录屏只在本机处理；不上传，无遥测。

## 从源码构建

需要稳定版 Rust 工具链。跨平台 core crate 可在任意受支持平台构建与测试；Windows/macOS 平台层与安装包由 CI 在对应系统构建。

```sh
cargo build --release
cargo test
```

## 许可

基于 MIT 许可发布，见 [LICENSE](LICENSE)。

---

<a id="english"></a>

# Capin

Capin is a screenshot, annotation, pin, OCR, scrolling-capture, and screen-recording tool for Windows 10+ and macOS 13+. The goal: simple, efficient, native.

[中文](#chinese) | English

## Features

- **Window capture** — drag to select a region by default; hold Alt to highlight and click a window, avoiding accidental captures; fixed ratios and cursor capture are available; click empty desktop for the full display.
- **Window finish** — composited rounded corners, shadow, padding and background canvas, with transparent, light, dark, gradient and wallpaper backgrounds plus transparent PNG; adjustable or disabled in settings.
- **Annotation** — rectangle, ellipse, arrow, highlighter, text, Gaussian blur, numbered badges, spotlight, object eraser, undo/redo; select an object to recolor or resize its stroke, and double-click text to edit it.
- **Pin** — image, text, file and color pins; always on top, opacity, zoom, rotation, multiple pins, click-through, restore after restart, annotation and OCR; image pins can recognize text and QR codes based on settings, with selectable and copyable text.
- **OCR** — region recognition, copy text, QR codes.
- **Scrolling capture** — automatic scrolling (Accessibility permission is required on macOS) with overlap matching; manual scrolling fallback.
- **Recording** — region or window recording on Windows and macOS (hold Alt and click a window); no audio and no camera; MP4 with the system H.264 encoder; pause, timer, cursor capture; the control bar stays out of frame.
- **System** — tray, global hotkey, clipboard, autosave, launch at login; screenshots save to a `Capin` folder in the system Pictures directory and recordings to a `Capin` folder in the system Videos directory, both configurable in settings; settings follow the system appearance and accent color; Chinese and English UI, Chinese by default; no upload, no telemetry.

## Install

Download the matching package from [Releases](https://github.com/fynhq/capin/releases):

| Platform | Installer | Portable |
|---|---|---|
| Windows x64 | `Capin_Windows_x64_vX.Y.Z_setup.exe` | `Capin_Windows_x64_vX.Y.Z.zip` |
| Windows arm64 | `Capin_Windows_arm64_vX.Y.Z_setup.exe` | `Capin_Windows_arm64_vX.Y.Z.zip` |
| macOS arm64 | `Capin_macOS_arm64_vX.Y.Z.dmg` | — |
| macOS x64 | `Capin_macOS_x64_vX.Y.Z.dmg` | — |

- The Windows installer is per-user and needs no administrator rights; the portable zip runs `Capin.exe` directly.
- On macOS, open the dmg, drag `Capin.app` to `/Applications` (or `~/Applications`), and launch the app rather than the binaries inside `Contents/MacOS`.
- If launch is blocked: on Windows choose **More info → Run anyway**; on macOS run:

```sh
xattr -d -r com.apple.quarantine /Applications/Capin.app
```

- Upgrade by running the new Windows installer or replacing the portable folder; on macOS replace the app. Uninstall from Windows Settings > Apps, or delete the portable folder; on macOS delete the app. Settings remain in the user data directory and captures stay in the Pictures/Videos folders until removed manually.
- Each release includes `SHA256SUMS`; verify with `sha256sum -c SHA256SUMS` (on macOS use `shasum -a 256 -c SHA256SUMS`).

## Usage

- Start a capture from the tray menu or the global hotkey; drag to select a region by default, or hold Alt and click a window, and press Esc to cancel.
- During selection, press `R` to cycle Free/1:1/4:3/16:9; the magnifier shows the color under the cursor and the selection size; press `Shift` to switch between hex and RGB, and `C` to copy the current value.
- Global hotkeys: `Win/Cmd+Shift+S` captures, `Win/Cmd+Shift+D` pins; both can be changed in settings.
- In the editor, copy and save use the system shortcuts `Ctrl/Cmd+C` and `Ctrl/Cmd+S`.
- The tray menu offers capture, recording, scrolling capture, OCR, the screenshots folder, settings, release pinned windows, restart, and quit; closing a window keeps Capin in the tray.
- The editor supports drawing, text, numbered badges, spotlight, blur, copy, save, and pin; select an existing object to recolor or resize its stroke, and double-click text to edit it again.
- Configure window corners, shadow, padding and background under Settings → Capture → Window capture; these styles apply to window captures only. The default is a transparent PNG with alpha.
- Pin windows can stay on top, change opacity and zoom, and rotate image pins clockwise by 90 degrees; no action bar is shown by default. Left-click to move or select text, and right-click for rotate, edit, copy, save and close (plus click-through). After click-through, use Release pinned windows in the tray menu; pins restore after a restart.
- Image pins can recognize text and QR codes according to Settings → Capture → Pins: drag over text to select it and release to copy; double-click to select a word; drag any non-text area with the left button to move the pin.
- Recording encodes MP4 with the system H.264 encoder and includes the cursor by default (disable it in Settings → Recording). Drag a region or hold Alt and click a window to start; pause while recording, and the file is saved on stop.
- Captures and recordings stay local; no upload, no telemetry.

## Build from source

Requires a stable Rust toolchain. Cross-platform core crates build and test on any supported host; Windows/macOS platform layers and installers are built by CI on the matching OS.

```sh
cargo build --release
cargo test
```

## License

Released under the MIT License. See [LICENSE](LICENSE).
