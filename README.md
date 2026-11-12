<p align="center"><img src="assets/icon/capin_icon_1024.png" width="128" alt="Capin logo"></p>

<a id="chinese"></a>

# Capin

Capin 是面向 Windows 10+ 与 macOS 13+ 的截图、标注、贴图、OCR、长截图与录屏工具。目标简单、高效、类原生。

中文 | [English](#english)

## 功能

- **截图**：区域、窗口、元素、任务栏、贴图与设置页；悬停识别窗口/元素，滚轮切换层级，拖拽自由选区，桌面空白单击截整屏；支持固定比例、光标捕捉与 F5 刷新。Windows 使用 UI Automation，macOS 使用 Accessibility，未授权时回落窗口级。
- **窗口质感**：窗口截图固定系统风格阴影、内边距和 1px 主题边框，背景透明或白色；macOS 保留原生效果。区域截图可单独设置圆角与阴影，并延续到后续截图。
- **标注**：矩形/椭圆、直线/折线箭头、荧光笔、文字、高斯模糊、序号、聚光灯与撤销重做；点击对象即可选中，支持移动、缩放、复制粘贴、删除、改色与线宽。区域截图以冻结全屏为画布，标注后可拖选区边/角手柄调整导出范围，框外标注保留但不显示；贴图编辑支持裁剪、旋转与翻转，长截图只裁剪并支持缩放/平移。
- **贴图**：图片/文本贴图，置顶、透明度、平滑缩放、旋转、多贴、鼠标穿透、重启恢复、标注与自动 OCR；保留最近 7 天最多 50 张截图历史，可按贴图键逐张回看。
- **OCR**：贴图自动识别文字与二维码，结果可划选复制；Windows x64 使用随包 PaddleOCR，ARM64 或模型失败回落系统 OCR，macOS 使用 Vision；仅在本机处理。
- **长截图**：手动滚动、自动接图，重叠匹配与固定顶栏裁除；不注入滚轮，不自动滚动页面。
- **录屏**：区域/窗口录制，MP4（系统 H.264，固定 30 fps，自动码率），无音频与摄像头；倒计时、暂停/继续、计时、光标录入与演示高亮；录制条和选区边框不入镜。
- **系统**：托盘、全局热键、剪贴板、自动保存、开机自启、中英文界面；截图与录屏分目录保存，共用文件名模板；跟随系统亮暗与强调色；不上传，无遥测。

## 资源占用

Windows 11：后台常驻内存约 14 MB；截图操作期间约 100–150 MB，截图完成后逐步回落。

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

- macOS 每次安装新版本后，先在终端清除旧授权，再打开 Capin「设置 → 通用 → 权限」重新授权（屏幕录制授权后需重启 Capin）：

```sh
tccutil reset Accessibility io.github.fynhq.capin
tccutil reset ScreenCapture io.github.fynhq.capin
```

- 升级：Windows 运行新安装包覆盖安装，或替换绿色包目录；macOS 替换应用。卸载：Windows 在「设置 → 应用」中卸载，或直接删除绿色包目录；macOS 删除应用。配置文件保留在用户目录，截图与录屏保存在图片/视频目录，按需手动删除。
- 每个 Release 附带 `SHA256SUMS`；下载后可在包目录执行 `sha256sum -c SHA256SUMS`（macOS 用 `shasum -a 256 -c SHA256SUMS`）。

## 使用

- **开始截图**：单击托盘图标、托盘菜单「开始」或全局热键。悬停选窗口/元素，滚轮调整层级，拖拽选区域，桌面空白单击截整屏。按住 Ctrl 进入窗口模式，单击直接标注；Esc、Cmd/Ctrl+W/Q 或再次按截图快捷键取消，右键确认标注、不退出。macOS 元素识别需在「系统设置 → 隐私与安全性 → 辅助功能」授权 Capin。
- **选区与标注**：按 `R` 切换自由/1:1/4:3/16:9；放大镜显示像素颜色与尺寸，`Shift` 切换十六进制/RGB，`C` 复制色值。框选后在 overlay 内直接标注；区域截图在未选中对象且未使用标注工具时可拖选区边缘调整导出范围，框外标注保留但不显示；双击或 Enter 复制；空格隐藏工具栏，F1 快捷键，F5 只刷新底图。
- **编辑器**：复制/保存用 `Ctrl/Cmd+C`、`Ctrl/Cmd+S`；选中对象后可移动、缩放、复制粘贴、删除、改属性；右键或 Esc 退出当前工具。贴图可裁剪、旋转、翻转；长截图可裁剪，并用 `Ctrl/Cmd+滚轮` 缩放、`Space+拖动` 平移、`Ctrl/Cmd+0/1` 适应/100%。
- **贴图**：置顶、透明度、缩放、旋转、多贴、穿透；空白或边缘拖动，Alt 任意位置拖动；Esc 或中键关闭；未划选文字时 `Ctrl/Cmd+C` 复制整图、`Ctrl/Cmd+S` 保存图片；右键菜单含旋转、编辑、复制、保存、复制二维码、穿透与关闭；按贴图键回看最近 7 天历史。
- **录屏**：框选或悬停窗口后点工具栏录屏；3 秒倒计时，F9 暂停/继续，F10 结束；默认录入光标，演示光标可在设置开启并选色；停止后保存单个 MP4，可打开文件夹。
- **设置**：保存目录、文件名模板、热键、开机自启、语言、标注颜色/线宽、贴图 OCR、录屏光标与高亮均在设置页；所有内容仅在本机处理。

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

- **Capture** — regions, windows, elements, taskbar, pins and settings; hover detection, wheel to change level, free drag, empty-desktop click for the full display; fixed ratios, cursor capture and F5 refresh. Windows uses UI Automation; macOS uses Accessibility and falls back to whole windows without permission.
- **Window finish** — system-style shadow, padding and a 1px theme border over a transparent or white background; macOS keeps its native presentation. Region captures can set corner radius and shadow, kept for later captures.
- **Annotation** — rectangle/ellipse, straight/polyline arrows, highlighter, text, Gaussian blur, numbered badges, spotlight and undo/redo; click an object to select, move, resize, copy/paste, delete, recolor or change width. Region captures annotate the frozen full display and can resize the export frame afterwards; objects outside the frame stay in the document but are hidden and return when the frame grows. Pins support crop/rotate/flip; scrolling captures support crop plus zoom/pan.
- **Pins** — image/text pins with always-on-top, opacity, smooth zoom, rotation, multiple pins, click-through, restart restore, annotation and automatic OCR; keep the last 7 days / 50 screenshot exports and walk them with the pin hotkey.
- **OCR** — text and QR recognition on pins with selectable copy; Windows x64 uses bundled PaddleOCR, ARM64 or model failure falls back to system OCR, macOS uses Vision; local only.
- **Scrolling capture** — manual scrolling with automatic stitching, overlap matching and fixed-header trimming; no wheel injection.
- **Recording** — region/window MP4 via the system H.264 encoder at 30 fps and automatic bitrate; no audio or camera; countdown, pause/resume, timer, cursor and demo highlight; the control bar and selection border stay out of frame.
- **System** — tray, global hotkeys, clipboard, autosave, launch at login, Chinese/English UI; separate Pictures/Videos folders and a shared filename template; follows system appearance and accent; no upload, no telemetry.

## Resource usage

Windows 11: about 14 MB of memory while resident in the tray; 100–150 MB during a capture, settling back down after the capture completes.

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

- On macOS, after each new version is installed, clear the old grants in Terminal, then re-authorize in Capin under Settings → General → Permissions (restart Capin after granting Screen Recording):

```sh
tccutil reset Accessibility io.github.fynhq.capin
tccutil reset ScreenCapture io.github.fynhq.capin
```

- Upgrade by running the new Windows installer or replacing the portable folder; on macOS replace the app. Uninstall from Windows Settings > Apps, or delete the portable folder; on macOS delete the app. Settings remain in the user data directory and captures stay in the Pictures/Videos folders until removed manually.
- Each release includes `SHA256SUMS`; verify with `sha256sum -c SHA256SUMS` (on macOS use `shasum -a 256 -c SHA256SUMS`).

## Usage

- **Start a capture** — tray, menu or hotkey; hover to pick a window/element, scroll to change level, drag a region, or click empty desktop for the full display. Hold Ctrl for window-only mode and click to annotate; Esc/Cmd/Ctrl+W/Q cancels; right-click confirms annotation without exiting. macOS element detection needs Accessibility permission.
- **Selection and annotation** — press `R` to cycle ratios; `Shift` switches the color format and `C` copies the value; annotate in the overlay. Region captures expose edge/corner handles to resize the export frame after annotating; outside objects stay but are hidden. Double-click or Enter copies; Space hides the toolbar, F1 shows shortcuts, F5 refreshes only the base image.
- **Editor** — `Ctrl/Cmd+C/S` copies/saves; objects can be moved, resized, copied/pasted, deleted and restyled; right-click or Esc leaves the current tool. Pins support crop/rotate/flip; scrolling captures support crop and `Ctrl/Cmd+wheel` zoom, `Space+drag` pan, `Ctrl/Cmd+0/1` fit/100%.
- **Pins** — always on top, opacity, zoom, rotation, multiple pins, click-through; drag from blank areas/edges or Alt+drag anywhere; Esc/middle-click closes; `Ctrl/Cmd+C/S` copies/saves the whole pin when no text is selected; the right-click menu offers rotate, edit, copy, save, copy QR, click-through and close; the pin hotkey walks the last 7 days of history.
- **Recording** — select a region or hover a window and click record; 3-second countdown, F9 pause/resume, F10 stop; cursor is captured by default and the demo highlight can be enabled in settings; stop saves one MP4 and can open its folder.
- **Settings** — save folders, filename template, hotkeys, launch at login, language, annotation colors/widths, pin OCR and recording cursor/highlight live in Settings. Everything stays local.

## Build from source

Requires a stable Rust toolchain. Cross-platform core crates build and test on any supported host; Windows/macOS platform layers and installers are built by CI on the matching OS.

```sh
cargo build --release
cargo test
```

## License

Released under the MIT License. See [LICENSE](LICENSE).
