<p align="center"><img src="assets/icon/capin_icon_1024.png" width="128" alt="Capin logo"></p>

<a id="chinese"></a>

# Capin

Capin 是面向 Windows 10+ 与 macOS 13+ 的截图、标注、贴图、OCR、长截图与录屏工具。目标简单、高效、类原生。

中文 | [English](#english)

## 功能

- **窗口截图**：悬停自动识别窗口/元素，滚轮在元素、父级与整窗之间调整；拖拽自由选区，桌面空白处单击截取整屏；支持固定比例与光标捕捉。Windows 使用 UI Automation，macOS 使用 Accessibility；未授权时回落到窗口级。
- **窗口质感**：窗口截图合成圆角、阴影、内边距与背景画布，支持透明、浅色、深色、渐变、壁纸背景与透明 PNG；可在设置中调整或关闭。
- **标注**：矩形（可调圆角、实心/线框，实心固定不透明）、椭圆（实心/线框）、箭头（首尾箭头可选，默认尾部）、折线箭头（依次点击端点，双击/Enter 完成）、曲线箭头（拖拽生成，控制点可调）、荧光笔（框选）、文字（字号/背景/描边/对齐）、高斯模糊（可调强度）、序号（圆形/方形/纯数字）、聚光灯（可调强度）、撤销重做；支持裁剪、90° 旋转、水平/垂直翻转与画布缩放/平移；颜色在编辑器工具栏选择（含黑、白），设置中可切换所有工具统一颜色或各工具独立；线宽、字号、圆角、模糊、填充、箭头与聚光灯强度均持久化；选中对象可拖手柄缩放、移动、复制粘贴、删除或改颜色/线宽，双击文字可再编辑；删除序号后自动续排。
- **贴图**：图片/文本/文件/颜色贴图，置顶、透明度、缩放、旋转、多贴、鼠标穿透、重启恢复、贴图标注与 OCR；图片贴图可按设置自动识别文字与二维码，文字可直接划选并自动复制。最近 7 天保留最多 50 张截图历史（仅最终导出图），重复按贴图快捷键可循环贴出历史截图；贴图本身不写入历史。
- **OCR**：框选识别、复制文本、二维码。
- **长截图**：自动滚动（macOS 需辅助功能权限）+ 重叠匹配与固定顶栏裁除；手动滚动兜底。
- **录屏**：Windows 与 macOS 区域或窗口录制（悬停窗口后单击），不采集声音与摄像头；MP4（系统 H.264 编码）或 WebP（libwebp 软件编码，固定 30 fps，输出最高 1080p）；开始前 3 秒倒计时；低/标准/高三档画质；可选“分段恢复”，每 10 秒保存可播放片段；暂停、计时、光标录入；录制条不入镜。
- **系统**：托盘、全局热键、剪贴板、自动保存、开机自启；截图默认保存到系统图片目录的 `Capin` 文件夹，录屏默认保存到系统视频目录的 `Capin` 文件夹，可在设置中修改；截图文件名模板默认 `Capin_{date}_{time}`，冲突自动加序号；设置页跟随系统亮暗与强调色；界面中英文，默认中文；不上传，无遥测。

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

- 通过托盘菜单的「开始」或全局热键开始截图；悬停自动识别窗口/元素，滚轮调整识别范围，拖拽自由选区，Esc、Cmd/Ctrl+W、Cmd/Ctrl+Q 或右键取消。macOS 的元素识别与长截图自动滚动需要在「系统设置 → 隐私与安全性 → 辅助功能」中授权 Capin；授权后如未生效，请重启 Capin。「设置 → 通用 → 权限」会显示屏幕录制与辅助功能状态并提供系统设置入口。
- 截图时按 `R` 切换自由/1:1/4:3/16:9 比例；放大镜显示光标下的颜色和选区尺寸，按 `Shift` 切换十六进制/RGB，按 `C` 复制当前格式的颜色值。
- 全局热键：`Win/Cmd+Shift+S` 截图，`Win/Cmd+Shift+D` 贴图；可在设置中修改。
- 编辑器内复制/保存使用系统快捷键 `Ctrl/Cmd+C`、`Ctrl/Cmd+S`；选中标注后 `Ctrl/Cmd+C` 复制对象、`Ctrl/Cmd+V` 粘贴、`Delete` 删除，拖动对象可移动。
- 托盘右键菜单提供开始（截图）、设置、重启与退出；有可恢复录制或穿透贴图时额外显示对应恢复项。关闭窗口后应用留在托盘。
- 框选后在截图 overlay 内直接标注，不另开编辑窗口；选区上方信息栏显示位置/尺寸/比例/效果/刷新，四周控制点可继续调整，双击或 Enter 复制；工具栏的更多菜单可进入长截图、录屏、OCR、裁剪、旋转与翻转；截图浮层跟随系统亮暗，使用与 Windows/macOS 原生一致的中性表面、边框、阴影与系统强调色；空格隐藏工具栏，F1 查看快捷键，F5 刷新截图。标注支持绘制、文字、序号、聚光灯、模糊，矩形/椭圆可切换实心与线框，矩形可调圆角，文字可调字号/背景/描边/对齐，箭头可配首尾，折线箭头依次点击端点并以双击/Enter 完成，曲线箭头拖拽生成后可拖控制点调整，序号可选圆形/方形/纯数字，模糊与聚光灯可调强度，荧光笔框选色块，复制、保存或贴图后关闭截图页；选中已有对象可拖手柄缩放、移动、复制粘贴、删除、改颜色与线宽，双击文字可再次编辑。独立编辑窗口（贴图标注、长截图结果）支持 Ctrl/Cmd+滚轮缩放、Space+左键拖动平移、Ctrl/Cmd+0 适应窗口与 Ctrl/Cmd+1 显示 100%；裁剪拖选区域后按 Enter 应用、Esc 取消；旋转/翻转会合并标注，可用撤销恢复。截图内联标注不提供画布缩放。
- 窗口截图的圆角、阴影、留白与背景在「设置 → 截图 → 窗口截图」中调整，这些样式只作用于窗口截图；区域截图可在选区信息栏的“效果”中单独开启圆角、阴影与边框。默认透明背景并输出带 Alpha 的 PNG。标注颜色在编辑器工具栏选择；「设置 → 截图 → 标注」只控制各工具是否统一颜色。截图文件名模板在「设置 → 截图 → 截图保存位置」下修改，默认 `Capin_{date}_{time}`，冲突自动加序号。
- 贴图窗口可置顶、调整透明度与缩放，图片贴图可顺时针旋转 90°；默认不显示操作栏，左键移动或划选文字，右键菜单可旋转、编辑、复制、保存、关闭；也可开启鼠标穿透。开启穿透后从托盘选择「恢复贴图交互」恢复操作；贴图会在重启后恢复。再次按贴图快捷键可循环贴出最近 7 天的截图历史（最多 50 张，仅最终导出图）。
- 图片贴图可按「设置 → 截图 → 贴图」中的开关自动识别文字与二维码：在文字上拖动即可划选，松开后自动复制；双击选择单词；在非文字区域按住左键拖动可移动贴图。
- 录屏可选 MP4（系统 H.264）或 WebP（软件编码，固定 30 fps、最高 1080p）；默认包含光标，可在「设置 → 录屏」中关闭；开始前有 3 秒倒计时；「设置 → 录屏」可选择低/标准/高画质；开启“分段恢复”后每 10 秒保存一个可播放片段，正常停止输出会话文件夹，崩溃后可从托盘“恢复未完成录制”打开；框选后点击工具栏的录屏按钮开始，录制过程可暂停，停止后自动保存。
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
- **Window finish** — composited rounded corners, shadow, padding and background canvas, with transparent, light, dark, gradient and wallpaper backgrounds plus transparent PNG; adjustable or disabled in settings.
- **Annotation** — rectangle (adjustable corner radius), ellipse, arrow (optional start/end heads, end head by default), polyline arrow (click points, double-click/Enter to finish), curve arrow (drag; control point editable), highlighter, text (font size/background/outline/alignment), Gaussian blur (adjustable intensity), numbered badges (circle/square/plain), spotlight, undo/redo; crop, rotate 90°, flip horizontally/vertically, and zoom/pan the canvas; choose colors in the editor toolbar, and use settings to make all tools share one color or keep them independent; select an object to resize with handles, move, copy, paste, delete, recolor, or resize its stroke; double-click text to edit it. Deleting a numbered badge renumbers the rest.
- **Pin** — image, text, file and color pins; always on top, opacity, zoom, rotation, multiple pins, click-through, restore after restart, annotation and OCR; image pins can recognize text and QR codes based on settings, with selectable and copyable text. The last 7 days of final screenshot exports (up to 50) are kept; pressing the pin hotkey again cycles through them. Pins themselves are not added to history.
- **OCR** — region recognition, copy text, QR codes.
- **Scrolling capture** — automatic scrolling (Accessibility permission is required on macOS) with overlap matching; manual scrolling fallback.
- **Recording** — region or window recording on Windows and macOS (hover a window and click); no audio and no camera; MP4 with the system H.264 encoder or WebP with the libwebp software encoder (fixed 30 fps, up to 1080p); a 3-second countdown; low/standard/high quality; optional segmented recovery with a playable segment every 10 seconds; pause, timer, cursor capture; the control bar stays out of frame.
- **System** — tray, global hotkey, clipboard, autosave, launch at login; screenshots save to a `Capin` folder in the system Pictures directory and recordings to a `Capin` folder in the system Videos directory, both configurable in settings; the screenshot filename template defaults to `Capin_{date}_{time}` with automatic numeric suffixes on conflict; settings follow the system appearance and accent color; Chinese and English UI, Chinese by default; no upload, no telemetry.

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

- Start a capture from Start in the tray menu or the global hotkey; hover to detect a window or element, scroll to adjust the target, drag for a free region, and press Esc, Cmd/Ctrl+W, Cmd/Ctrl+Q or right-click to cancel. On macOS, element detection and automatic scrolling capture require granting Capin under System Settings → Privacy & Security → Accessibility; restart Capin if detection does not take effect. Settings → General → Permissions shows screen-recording and accessibility status with shortcuts to System Settings.
- During selection, press `R` to cycle Free/1:1/4:3/16:9; the magnifier shows the color under the cursor and the selection size; press `Shift` to switch between hex and RGB, and `C` to copy the current value.
- Global hotkeys: `Win/Cmd+Shift+S` captures, `Win/Cmd+Shift+D` pins; both can be changed in settings.
- In the editor, copy and save use the system shortcuts `Ctrl/Cmd+C` and `Ctrl/Cmd+S`; with an annotation selected, `Ctrl/Cmd+C` copies the object, `Ctrl/Cmd+V` pastes it, `Delete` removes it, and dragging moves it.
- The tray menu offers Start (capture), Settings, Restart and Quit; recovery entries appear only when an unfinished recording or a click-through pin needs them. Closing a window keeps Capin in the tray.
- After selecting a region, annotations happen in the capture overlay itself; no separate editor window opens. The info bar above the selection shows position, size, ratio, effects and refresh, while the handles let you keep adjusting the region; double-click or Enter copies. The toolbar is a compact palette next to the selection, and its More menu starts scrolling capture, recording, OCR, crop, rotate 90°, horizontal/vertical flip. Capture chrome follows the system light/dark appearance with native-style neutral surfaces, borders, shadows and the system accent; Space hides the toolbar, F1 shows shortcuts, and F5 refreshes the capture. Copy, save or pin closes the capture page. The editor supports drawing, text, numbered badges, spotlight and blur with adjustable intensity, rectangle corner radius, text size/background/outline/alignment, start/end arrow heads, polyline and curve arrows, numbered badge styles, crop, rotate, flip, copy, save, and pin; select an existing object to resize with handles, move, copy, paste, delete, recolor or resize its stroke, and double-click text to edit it again. Standalone editor windows (pin annotation, scroll results) support Ctrl/Cmd+wheel zoom, Space+left-drag pan, Ctrl/Cmd+0 to fit, and Ctrl/Cmd+1 for 100%. In crop mode, drag a region and press Enter to apply or Esc to cancel; rotate/flip flattens annotations and can be undone. The inline capture editor does not zoom.
- Configure window corners, shadow, padding and background under Settings → Capture → Window capture; these styles apply to window captures only. Region captures can enable corner radius, shadow and border from the Effects menu in the selection info bar. The default is a transparent PNG with alpha. Choose annotation colors in the editor toolbar; Settings → Capture → Annotations only controls whether all tools share one color. The screenshot filename template lives under Settings → Capture → Screenshot folder and defaults to `Capin_{date}_{time}` with automatic numeric suffixes on conflict.
- Pin windows can stay on top, change opacity and zoom, and rotate image pins clockwise by 90 degrees; no action bar is shown by default. Left-click to move or select text, and right-click for rotate, edit, copy, save and close (plus click-through). After click-through, use Release pinned windows in the tray menu; pins restore after a restart. Press the pin hotkey again to cycle through the last 7 days of final screenshot exports (up to 50).
- Image pins can recognize text and QR codes according to Settings → Capture → Pins: drag over text to select it and release to copy; double-click to select a word; drag any non-text area with the left button to move the pin.
- Recording saves MP4 with the system H.264 encoder or WebP with the software encoder (fixed 30 fps, up to 1080p), and includes the cursor by default (disable it in Settings → Recording). A 3-second countdown starts the capture; low/standard/high quality is available in Settings → Recording. Enable segmented recovery to save a playable segment every 10 seconds; normal stop produces a session folder, and a crash can be recovered from the tray. Select a region and click the record button in the toolbar; pause while recording, and the file is saved on stop.
- Captures and recordings stay local; no upload, no telemetry.

## Build from source

Requires a stable Rust toolchain. Cross-platform core crates build and test on any supported host; Windows/macOS platform layers and installers are built by CI on the matching OS.

```sh
cargo build --release
cargo test
```

## License

Released under the MIT License. See [LICENSE](LICENSE).
