<p align="center"><img src="assets/icon/capin_icon_1024.png" width="128" alt="Capin logo"></p>

<a id="chinese"></a>

# Capin

Capin 是面向 Windows 10+ 与 macOS 13+ 的截图、标注、贴图、OCR、长截图与录屏工具。目标简单、高效、类原生。

中文 | [English](#english)

## 功能

- **窗口截图**：悬停自动识别窗口/元素，窗口、元素与整屏使用不同描边和标签；滚轮在元素、父级与整窗之间调整；拖拽自由选区，桌面空白处单击截取整屏；按住 Ctrl 只识别窗口/整屏，单击直接进入标注；支持固定比例与光标捕捉。窗口、任务栏、贴图与设置页都可作为截图目标。Windows 使用 UI Automation，macOS 使用 Accessibility；未授权时回落到窗口级。
- **窗口质感**：窗口截图固定带系统风格阴影与内边距（默认 24，可选 24/48/96），边框按系统主题重绘为 1px（浅色 `#B6B6B6`），背景仅透明或白色（默认透明），不提供边框或阴影开关；macOS 保留系统原生窗口效果。区域截图可单独设置圆角与阴影，效果选择会保留到后续截图。
- **标注**：矩形工具（可切换矩形/椭圆子选项，矩形可调圆角，实心/线框，实心固定不透明）、箭头（填充矢量轮廓，非箭头端圆头，首尾箭头可选，可切换直线/折线子选项，折线依次点击端点，双击/Enter 完成）、荧光笔（框选，纯色高透明度）、文字（字号/背景/描边/对齐）、高斯模糊（可调强度）、序号（圆形）、聚光灯（可调强度）、撤销重做；不单设选择工具，点击已有对象即选中。Esc 先取消当前对象/工具，再按才退出；未选中对象且未使用标注工具时可拖动图像边缘重新裁剪（窗口/全屏截图除外）；拖动显示裁剪预览、松开应用；Esc 取消标注后恢复区域手柄。贴图标注支持裁剪、90° 旋转、水平/垂直翻转，长截图结果只支持裁剪；截图内联编辑不提供这些操作；长截图结果支持画布缩放/平移；颜色在编辑器工具栏选择（含黑、白与自定义色值），设置中可切换所有工具统一颜色或各工具独立；线宽、字号、圆角、模糊、填充、箭头与聚光灯强度均持久化；选中对象可拖手柄缩放、移动、复制粘贴、删除或改颜色/线宽，双击文字可再编辑；未在绘制文字/折线/裁剪时，右键退出当前标注工具并回到选择，等同 Esc 的第一步；切换工具后点击已有对象仍可直接选中并拖动，按住 Shift 拖拽把矩形约束为正方形、椭圆约束为圆形；删除序号后自动续排。
- **贴图**：图片/文本贴图，置顶、透明度、缩放、旋转、多贴、鼠标穿透、重启恢复、贴图标注与 OCR；图片贴图可按设置自动识别文字与二维码，从空白处或边缘拖动移动、按住 Alt 可从任意位置拖动，在文字上直接拖动划选并用系统复制快捷键复制，未划选时 `Ctrl/Cmd+C` 复制整张贴图、`Ctrl/Cmd+S` 保存图片。滚轮以窗口中心平滑缩放；最近 7 天保留最多 50 张截图历史（仅最终导出图，同一张图只保留最新一条），关闭贴图后再次按贴图快捷键先恢复刚关闭的贴图，再按才沿历史严格向上切换、到最早一张停止，剪贴板出现新图时改为贴新图；贴图本身不写入历史。区域与长截图贴图带系统阴影，窗口截图贴图保留图像内合成阴影；贴图窗口不绘制额外边框。
- **OCR**：贴图后自动识别文字与二维码，结果可划选并用系统复制快捷键复制；英文行使用英文标点，复制时按版式自动判断软换行与硬换行；不提供独立框选 OCR。
- **长截图**：用户手动滚动，滚动时自动接图（默认开启，无手动截取与自动开关），重叠匹配与固定顶栏裁除；选区显示 3px 红色边框；不注入滚轮、不自动滚动页面。
- **录屏**：Windows 与 macOS 区域或窗口录制（悬停窗口后单击），不采集声音与摄像头；MP4（系统 H.264 编码，固定 30 fps、自动码率）；开始前 3 秒倒计时；F9 暂停/继续、F10 结束、计时、光标录入、演示光标圆形高亮（颜色可设置，录制中实时显示）；录制中显示 3px 红色选区边框；停止后直接保存为单个视频文件，完成提示带「打开文件夹」并在 3 秒后自动消失；录制条与边框不入镜；MP4 文件属性写入编码人员 `Capin` 与作者 URL。
- **系统**：托盘、全局热键、剪贴板、自动保存、开机自启；截图默认保存到系统图片目录的 `Capin` 文件夹，录屏默认保存到系统视频目录的 `Capin` 文件夹，可在设置中修改；截图与录屏共用文件名模板默认 `Capin_{date}_{time}`，冲突自动加序号；设置页跟随系统亮暗与强调色；界面中英文，默认中文；不上传，无遥测。

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

- **开始截图**：单击托盘图标、托盘菜单「开始」或全局热键。悬停自动识别窗口/元素，滚轮调整范围，拖拽自由选区，桌面空白单击截整屏；Esc、Cmd/Ctrl+W/Q 或再次按截图快捷键取消，右键确认标注、不退出。按住 Ctrl 进入窗口模式，单击直接进入标注。macOS 元素识别需在「系统设置 → 隐私与安全性 → 辅助功能」授权 Capin；「设置 → 通用 → 权限」显示权限状态与入口。
- **选区与画布**：按 `R` 切换自由/1:1/4:3/16:9；放大镜以像素矩阵显示颜色与尺寸，`Shift` 切换十六进制/RGB，`C` 复制色值。框选后在 overlay 内直接标注，不另开编辑器；信息栏可继续调整；未选中对象且未使用标注工具时显示区域边缘手柄，Esc 取消标注后恢复；双击或 Enter 复制；工具栏提供长截图与录屏，空格隐藏工具栏（底部可恢复），F1 快捷键，F5 刷新。
- **编辑器**：复制/保存用 `Ctrl/Cmd+C`、`Ctrl/Cmd+S`；选中对象后可复制、粘贴、删除、拖动；未在绘制文字/折线/裁剪时右键退出当前标注工具，等同 Esc 的第一步。标注工具与对象编辑见「功能」。Esc 先取消选中对象/当前工具，再按退出；未选中对象时可拖动图像边缘重新裁剪区域（窗口/全屏截图除外）。贴图标注与长截图结果同样原位编辑：贴图可裁剪、旋转、翻转，长截图只裁剪；长截图支持 Ctrl/Cmd+滚轮缩放、Space+拖动、Ctrl/Cmd+0 适应、Ctrl/Cmd+1 100%；裁剪 Enter 应用、Esc 取消，旋转/翻转会合并标注且可撤销。
- **窗口与区域质感**：窗口截图固定系统风格阴影与内边距（默认 24，可选 24/48/96），背景仅透明/白色，无边框/阴影开关；区域截图在信息栏「效果」中开启圆角与阴影，选择会保留到后续截图。标注颜色在编辑器工具栏选择；「设置 → 截图 → 标注」只控制是否统一颜色。文件名模板在「设置 → 通用」，默认 `Capin_{date}_{time}`，冲突自动加序号。
- **热键与托盘**：Windows `Ctrl+Alt+S` 截图、`Ctrl+Alt+D` 贴图；macOS `Ctrl+Cmd+S`/`Ctrl+Cmd+D`；可在设置修改。托盘单击开始截图，右键菜单含开始、设置、重启、退出；有穿透贴图时显示恢复入口；关闭窗口后留在托盘。
- **贴图**：置顶、透明度、居中平滑缩放、旋转、多贴、穿透；默认无操作栏，空白/边缘拖动移动，Alt 任意位置拖动；Esc 或中键关闭；未划选文字时 `Ctrl/Cmd+C` 复制整张贴图、`Ctrl/Cmd+S` 保存图片；右键菜单可旋转、编辑、复制、保存、复制二维码、穿透、关闭；穿透后按 Esc 或从托盘「恢复贴图交互」恢复，未穿透时 Esc 或中键关闭；重启后恢复。关闭贴图后再次按贴图快捷键先恢复刚关闭的贴图，再按才沿最近 7 天最多 50 张截图历史向上切换、到最早一张停止，剪贴板有新图时贴新图。
- **贴图 OCR**：按「设置 → 截图 → 贴图」开关自动识别文字与二维码；文字上拖动划选，松开后用系统复制快捷键复制；双击选词；空白/边缘或 Alt+拖动移动贴图。
- **录屏**：MP4、30 fps、自动码率；默认包含光标，可在「设置 → 录屏」关闭；可开启演示光标（浅黄/浅蓝/浅紫/浅绿预设 + 自定义色，录制时实时显示）；3 秒倒计时；框选后点工具栏录屏，录制区显示 3px 红色边框；F9 暂停/继续、F10 结束；停止后保存单个视频文件，完成提示可打开文件夹并在 3 秒后消失。
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

- **Window capture** — hover to detect windows and elements automatically; scroll to switch between an element, its parent, and the whole window. Drag for a free region or click empty desktop for the full display; fixed ratios and cursor capture are available. Windows uses UI Automation; macOS uses Accessibility and falls back to whole windows when permission is missing.
- **Window finish** — window captures composite a system-style shadow and padding (24 by default, 24/48/96 available) over a transparent or white background (transparent by default); the 1px border is repainted from the system theme (light: `#B6B6B6`); no border or shadow toggle. Region captures can enable their own corner radius and shadow, and the choice is kept for later captures. macOS keeps its native window presentation.
- **Annotation** — rectangle tool with a rectangle/ellipse sub-option (adjustable corner radius for rectangles, solid or outline), arrow tool with optional start/end heads and a straight/polyline sub-option (click points, double-click/Enter to finish; the non-head end is rounded), highlighter (translucent pure color), text (font size/background/outline/alignment), Gaussian blur (adjustable intensity), circular numbered badges, spotlight, undo/redo; no separate select tool — clicking an existing object selects it; region-edge cropping is available only without a selected object and with no drawing tool active, shows a crop preview while dragging, and applies on release. Crop, rotate 90° and flip horizontally/vertically for pin annotations; in-place scroll-capture results support crop only (the capture page itself does not offer these operations), with canvas zoom/pan for scroll-capture results; choose colors in the editor toolbar, and use settings to make all tools share one color or keep them independent; click any existing object to select and drag it even while a drawing tool is active; hold Shift while drawing to constrain rectangles to squares and ellipses to circles; select an object to resize with handles, move, copy, paste, delete, recolor, or resize its stroke; double-click text to edit it; right-click leaves the current drawing tool and returns to Select like the first Escape press when no text, polyline or crop is in progress. Deleting a numbered badge renumbers the rest.
- **Pin** — image and text pins; always on top, opacity, zoom, rotation, multiple pins, click-through, restore after restart, annotation and OCR; Esc or middle-click closes a pin; image pins can recognize text and QR codes based on settings; drag to move the pin and hold Alt to drag from anywhere, or drag over text to select it, then copy with the system shortcut; with no text selected, `Ctrl/Cmd+C` copies the whole pin and `Ctrl/Cmd+S` saves the image. The last 7 days of final screenshot exports (up to 50, deduplicated) are kept; after closing a pin, pressing the pin hotkey reopens that same pin, and pressing it again walks the history from newest to oldest and stops at the oldest. Region and long-capture pins use the system window shadow; window-capture pins keep their composed shadow, and pin windows draw no extra border. Pins themselves are not added to history.
- **OCR** — automatic text and QR recognition on pins with selectable text; Latin lines keep ASCII punctuation, and copied text reflows wrapped lines while preserving real paragraph breaks; no separate region-OCR flow.
- **Scrolling capture** — the user scrolls manually while auto-capture keeps grabbing frames; overlap matching and fixed-header trimming; no wheel injection.
- **Recording** — region or window recording on Windows and macOS (hover a window and click); no audio and no camera; MP4 with the system H.264 encoder at a fixed 30 fps and automatic bitrate; a 3-second countdown; pause, timer, cursor capture; stop saves a single video file; the control bar stays out of frame; the MP4 metadata records `Capin` as the encoded-by value and sets the author URL.
- **System** — tray, global hotkey, clipboard, autosave, launch at login; screenshots save to a `Capin` folder in the system Pictures directory and recordings to a `Capin` folder in the system Videos directory, both configurable in settings; the shared screenshot/recording filename template defaults to `Capin_{date}_{time}` with automatic numeric suffixes on conflict; settings follow the system appearance and accent color; Chinese and English UI, Chinese by default; no upload, no telemetry.

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

- **Start a capture** — click the tray icon, choose Start from the tray menu, or use the global hotkey. Hover to detect windows/elements, scroll to adjust the target, drag for a free region, or click empty desktop for the full display. Press Esc, Cmd/Ctrl+W, Cmd/Ctrl+Q or the capture hotkey again to cancel; right-click confirms the annotation without exiting. Hold Ctrl for window-only detection and click to annotate. On macOS, grant Capin under System Settings → Privacy & Security → Accessibility; Settings → General → Permissions shows status and shortcuts.
- **Selection and canvas** — press `R` to cycle Free/1:1/4:3/16:9; the pixel-matrix magnifier shows color and size, `Shift` switches hex/RGB, `C` copies the value. Annotate in the overlay itself; no separate editor opens. The info bar keeps adjusting the region; edge handles appear only without a selected object and drawing tool, and return after Esc cancels annotation; double-click or Enter copies. The toolbar offers scrolling capture and recording; Space hides it (a chip restores it), F1 shows shortcuts, F5 refreshes.
- **Editor** — copy/save with `Ctrl/Cmd+C` and `Ctrl/Cmd+S`; select an object to copy, paste, delete or drag it; right-click leaves the current drawing tool and returns to Select like the first Escape press when no text, polyline or crop is in progress. Annotation tools and object editing are under Features. Pin annotations and scroll-capture results also edit in place: pins support crop, rotate and flip; scroll results support crop only. Scroll results support Ctrl/Cmd+wheel zoom, Space+drag pan, Ctrl/Cmd+0 to fit and Ctrl/Cmd+1 for 100%; crop applies with Enter and cancels with Esc; rotate/flip flattens annotations and can be undone.
- **Window and region finish** — window captures use a system-style shadow and padding (24 by default; 24/48/96 available) over a transparent or white background, with no border or shadow toggle. Region captures enable corner radius and shadow from Effects in the info bar, and the choice is kept for later captures. Choose annotation colors in the editor toolbar; Settings → Capture → Annotations only controls whether all tools share one color. The shared screenshot/recording filename template lives under Settings → General and defaults to `Capin_{date}_{time}` with automatic numeric suffixes.
- **Hotkeys and tray** — Windows: `Ctrl+Alt+S` capture, `Ctrl+Alt+D` pin; macOS: `Ctrl+Cmd+S`/`Ctrl+Cmd+D`; change them in settings. Click the tray icon to capture; right-click for Start, Settings, Restart and Quit, plus Release pinned windows when needed. Closing a window keeps Capin in the tray.
- **Pin** — always on top, opacity, zoom, rotation, multiple pins, click-through. No action bar by default; drag from blank areas/edges, hold Alt to drag from anywhere; Esc or middle-click closes. With no text selected, `Ctrl/Cmd+C` copies the whole pin and `Ctrl/Cmd+S` saves the image. The right-click menu offers rotate, edit, copy, save, copy QR, click-through and close; after click-through press Esc or use Release pinned windows in the tray, while Esc or middle-click closes an interactive pin. Pins restore after restart. After closing a pin, press the pin hotkey to reopen it; press again to walk the last 7 days of screenshot history (up to 50) from newest to oldest, or flash the current pin, depending on settings.
- **Pin OCR** — use Settings → Capture → Pins to recognize text and QR codes; drag over text to select, then copy with the system shortcut; double-click selects a word; drag elsewhere or Alt+drag to move the pin.
- **Recording** — MP4 at a fixed 30 fps with automatic bitrate; the cursor is included by default (disable in Settings → Recording). The demo cursor offers light yellow, light blue, light purple and light green presets plus a custom color, and appears live while recording. A 3-second countdown starts; click the record button in the toolbar, pause while recording, and stop to save one video file.
- Captures and recordings stay local; no upload, no telemetry.

## Build from source

Requires a stable Rust toolchain. Cross-platform core crates build and test on any supported host; Windows/macOS platform layers and installers are built by CI on the matching OS.

```sh
cargo build --release
cargo test
```

## License

Released under the MIT License. See [LICENSE](LICENSE).
