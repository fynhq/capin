<p align="center"><img src="assets/icon/capin_icon_1024.png" width="128" alt="Capin logo"></p>

<a id="chinese"></a>

# Capin

Capin 是面向 Windows 10+ 与 macOS 13+ 的截图、标注、贴图、OCR、长截图与录屏工具。目标简单、高效、类原生。

中文 | [English](#english)

## 功能

- **窗口截图**：悬停自动识别窗口/元素，窗口、元素与整屏使用不同描边和标签；滚轮在元素、父级与整窗之间调整；拖拽自由选区，桌面空白处单击截取整屏；按住 Ctrl 只识别窗口/整屏，单击直接进入标注；支持固定比例与光标捕捉。窗口、任务栏、贴图与设置页都可作为截图目标。Windows 使用 UI Automation，macOS 使用 Accessibility；未授权时回落到窗口级。
- **窗口质感**：窗口截图固定带系统风格阴影与内边距（默认 24，可选 24/48/96），背景可选透明/跟随系统/亮色/暗色，不提供边框或阴影开关；macOS 保留系统原生阴影。区域截图可单独设置圆角与阴影。
- **标注**：矩形工具（可切换矩形/椭圆子选项，矩形可调圆角，实心/线框，实心固定不透明）、箭头（首尾箭头可选，可切换直线/折线子选项，折线依次点击端点，双击/Enter 完成）、荧光笔（框选）、文字（字号/背景/描边/对齐）、高斯模糊（可调强度）、序号（圆形）、聚光灯（可调强度）、撤销重做；不单设选择工具，点击已有对象即选中。贴图标注支持裁剪、90° 旋转、水平/垂直翻转，长截图结果只支持裁剪；截图内联编辑不提供这些操作；长截图结果支持画布缩放/平移；颜色在编辑器工具栏选择（含黑、白与自定义色值），设置中可切换所有工具统一颜色或各工具独立；线宽、字号、圆角、模糊、填充、箭头与聚光灯强度均持久化；选中对象可拖手柄缩放、移动、复制粘贴、删除或改颜色/线宽，双击文字可再编辑；切换工具后点击已有对象仍可直接选中并拖动，按住 Shift 拖拽把矩形约束为正方形、椭圆约束为圆形；删除序号后自动续排。
- **贴图**：图片/文本贴图，置顶、透明度、缩放、旋转、多贴、鼠标穿透、重启恢复、贴图标注与 OCR；图片贴图可按设置自动识别文字与二维码，从空白处或边缘拖动移动、按住 Alt 可从任意位置拖动，在文字上直接拖动划选并用系统复制快捷键复制。最近 7 天保留最多 50 张截图历史（仅最终导出图），重复按贴图快捷键可恢复历史截图或闪烁当前贴图（可设置）；贴图本身不写入历史。贴图窗口只显示截图内容，不额外绘制边框或投影。
- **OCR**：贴图后自动识别文字与二维码，结果可划选并用系统复制快捷键复制；不提供独立框选 OCR。
- **长截图**：用户手动滚动，滚动时自动接图，重叠匹配与固定顶栏裁除；不注入滚轮、不自动滚动页面。
- **录屏**：Windows 与 macOS 区域或窗口录制（悬停窗口后单击），不采集声音与摄像头；MP4（系统 H.264 编码，固定 30 fps、自动码率）；开始前 3 秒倒计时；暂停、计时、光标录入、演示光标圆形高亮（颜色可设置）；停止后直接保存为单个视频文件；录制条不入镜。
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

- 单击托盘图标或通过托盘菜单「开始」、全局热键开始截图；悬停自动识别窗口/元素，滚轮调整识别范围，拖拽自由选区，Esc、Cmd/Ctrl+W、Cmd/Ctrl+Q 或再次按截图快捷键取消；右键用于确认标注，不再退出。macOS 的元素识别需要在「系统设置 → 隐私与安全性 → 辅助功能」中授权 Capin；授权后如未生效，请重启 Capin。「设置 → 通用 → 权限」会显示屏幕录制与辅助功能状态并提供系统设置入口。
- 截图时按 `R` 切换自由/1:1/4:3/16:9 比例；放大镜显示光标下的颜色和选区尺寸，按 `Shift` 切换十六进制/RGB，按 `C` 复制当前格式的颜色值。按住 Ctrl 进入窗口模式：光标变为相机，只识别窗口或整屏，单击直接进入标注。
- 全局热键：Windows 为 `Ctrl+Alt+S` 截图、`Ctrl+Alt+D` 贴图，macOS 为 `Cmd+Shift+S`/`Cmd+Shift+D`；可在设置中修改。Windows 的 `Win+Shift+S` 由系统截图工具占用，无法注册。
- 编辑器内复制/保存使用系统快捷键 `Ctrl/Cmd+C`、`Ctrl/Cmd+S`；选中标注后 `Ctrl/Cmd+C` 复制对象、`Ctrl/Cmd+V` 粘贴、`Delete` 删除，拖动对象可移动。
- 托盘单击图标直接开始截图；右键菜单提供开始（截图）、设置、重启与退出。有穿透贴图时额外显示恢复入口。关闭窗口后应用留在托盘。
- 框选后在截图 overlay 内直接标注，不另开编辑窗口；选区上方信息栏显示位置/尺寸/比例/效果/刷新，四周控制点可继续调整，双击或 Enter 复制；工具栏直接提供长截图与录屏，截图页不提供裁剪、旋转与翻转；截图浮层跟随系统亮暗，使用与 Windows/macOS 原生一致的中性表面、边框、阴影与系统强调色；空格隐藏工具栏（底部保留“显示工具栏”入口），F1 查看快捷键，F5 刷新截图。标注支持绘制、文字、序号、聚光灯、模糊，矩形工具可切换矩形/椭圆子选项，矩形/椭圆可切换实心与线框，按住 Shift 可画正方形/圆形，矩形可调圆角，文字可调字号/背景/描边/对齐，箭头工具可配首尾并切换直线/折线子选项，折线依次点击端点并以双击/Enter 完成，所有箭头非箭头端为圆头，模糊与聚光灯可调强度，荧光笔框选色块，复制、保存或贴图后关闭截图页；工具栏不单设选择按钮，点击已有对象即选中，切换工具后点击已有对象仍可直接选中、拖动或改属性，选中对象可拖手柄缩放、复制粘贴、删除、改颜色与线宽，双击文字可直接在图上再次编辑。贴图标注与长截图结果也在原位编辑层中完成，不另开独立编辑器；贴图标注可从更多菜单裁剪、旋转与翻转，长截图只提供裁剪；长截图结果支持 Ctrl/Cmd+滚轮缩放、Space+左键拖动平移、Ctrl/Cmd+0 适应窗口与 Ctrl/Cmd+1 显示 100%，普通截图与贴图编辑保持原尺寸。裁剪拖选区域后按 Enter 应用、Esc 取消；旋转/翻转会合并标注，可用撤销恢复。
- 窗口截图固定使用系统风格阴影与内边距（默认 24，可选 24/48/96），背景可选透明/跟随系统/亮色/暗色；不提供边框、阴影开关或自定义阴影颜色。区域截图可在选区信息栏的“效果”中开启圆角与阴影，选区与最终图片会同步变化。标注颜色在编辑器工具栏选择，调色板末尾的自定义色支持自定义色值与色相快选；「设置 → 截图 → 标注」只控制各工具是否统一颜色。截图文件名模板在「设置 → 通用」中修改，默认 `Capin_{date}_{time}`，冲突自动加序号；输入框失焦或按 Enter 自动保存。
- 贴图窗口可置顶、调整透明度与缩放，图片贴图可顺时针旋转 90°；默认不显示操作栏，从空白处或边缘拖动可移动贴图，按住 Alt 可从任意位置拖动；按 Esc 或单击鼠标中键可直接关闭贴图；右键为跟随系统亮暗的原生菜单，可旋转、编辑、复制、保存、关闭；也可开启鼠标穿透。开启穿透后从托盘选择「恢复贴图交互」恢复操作；贴图会在重启后恢复。再次按贴图快捷键时可在「设置 → 截图 → 贴图」的下拉选项中设置恢复历史贴图（循环最近 7 天的截图历史，最多 50 张）或闪烁当前贴图。贴图窗口不额外绘制边框或投影。
- 图片贴图可按「设置 → 截图 → 贴图」中的开关自动识别文字与二维码：在文字上直接拖动可划选，松开后用系统复制快捷键复制；双击选择单词；从空白处或边缘拖动（或 Alt+拖动）移动贴图。
- 录屏固定为 MP4（系统 H.264 编码），30 fps 与码率自动；默认包含光标，可在「设置 → 录屏」中关闭；可开启演示光标，为光标叠加半透明圆形底色，提供浅黄、浅蓝、浅紫、浅绿四种预设并支持自定义颜色（默认浅黄）；开始前有 3 秒倒计时；框选后点击工具栏的录屏按钮开始，录制过程可暂停，停止后直接保存为一个视频文件。
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
- **Window finish** — window captures always composite a system-style shadow and padding (24 by default, 24/48/96 available) over a transparent, system, light or dark background; no border or shadow toggle. macOS keeps the native window shadow.
- **Annotation** — rectangle tool with a rectangle/ellipse sub-option (adjustable corner radius for rectangles, solid or outline), arrow tool with optional start/end heads and a straight/polyline sub-option (click points, double-click/Enter to finish; the non-head end is rounded), highlighter, text (font size/background/outline/alignment), Gaussian blur (adjustable intensity), circular numbered badges, spotlight, undo/redo; no separate select tool — clicking an existing object selects it. Crop, rotate 90° and flip horizontally/vertically for pin annotations; in-place scroll-capture results support crop only (the capture page itself does not offer these operations), with canvas zoom/pan for scroll-capture results; choose colors in the editor toolbar, and use settings to make all tools share one color or keep them independent; click any existing object to select and drag it even while a drawing tool is active; hold Shift while drawing to constrain rectangles to squares and ellipses to circles; select an object to resize with handles, move, copy, paste, delete, recolor, or resize its stroke; double-click text to edit it. Deleting a numbered badge renumbers the rest.
- **Pin** — image and text pins; always on top, opacity, zoom, rotation, multiple pins, click-through, restore after restart, annotation and OCR; Esc or middle-click closes a pin; image pins can recognize text and QR codes based on settings; drag to move the pin and hold Alt to drag from anywhere, or drag over text to select it, then copy with the system shortcut. The last 7 days of final screenshot exports (up to 50) are kept; pressing the pin hotkey again cycles through them. The pin window draws no extra border or drop shadow. Pins themselves are not added to history.
- **OCR** — automatic text and QR recognition on pins with selectable text; no separate region-OCR flow.
- **Scrolling capture** — the user scrolls manually while auto-capture keeps grabbing frames; overlap matching and fixed-header trimming; no wheel injection.
- **Recording** — region or window recording on Windows and macOS (hover a window and click); no audio and no camera; MP4 with the system H.264 encoder at a fixed 30 fps and automatic bitrate; a 3-second countdown; pause, timer, cursor capture; stop saves a single video file; the control bar stays out of frame.
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

- Click the tray icon or choose Start from the tray menu, or use the global hotkey, to start a capture; hover to detect a window or element, scroll to adjust the target, drag for a free region, and press Esc, Cmd/Ctrl+W, Cmd/Ctrl+Q or the capture hotkey again to cancel; right-click confirms the current annotation instead of exiting. On macOS, element detection and automatic scrolling capture require granting Capin under System Settings → Privacy & Security → Accessibility; restart Capin if detection does not take effect. Settings → General → Permissions shows screen-recording and accessibility status with shortcuts to System Settings.
- During selection, press `R` to cycle Free/1:1/4:3/16:9; the magnifier shows the color under the cursor and the selection size; press `Shift` to switch between hex and RGB, and `C` to copy the current value.
- Global hotkeys: `Ctrl+Alt+S` captures and `Ctrl+Alt+D` pins on Windows; `Cmd+Shift+S`/`Cmd+Shift+D` on macOS. Both can be changed in settings. Windows reserves `Win+Shift+S` for Snipping Tool, so it cannot be registered.
- In the editor, copy and save use the system shortcuts `Ctrl/Cmd+C` and `Ctrl/Cmd+S`; with an annotation selected, `Ctrl/Cmd+C` copies the object, `Ctrl/Cmd+V` pastes it, `Delete` removes it, and dragging moves it.
- Left-click the tray icon to start a capture; right-click opens the menu with Start, Settings, Restart and Quit (plus Release pinned windows when a click-through pin needs it). Closing a window keeps Capin in the tray.
- After selecting a region, annotations happen in the capture overlay itself; no separate editor window opens. The info bar above the selection shows position, size, ratio, effects and refresh, while the handles let you keep adjusting the region; double-click or Enter copies. The toolbar is a compact palette next to the selection with direct scrolling-capture and recording buttons; the capture page does not offer crop, rotate 90° or horizontal/vertical flip. Capture chrome follows the system light/dark appearance with native-style neutral surfaces, borders, shadows and the system accent; Space hides the toolbar and a Show toolbar chip restores it, F1 shows shortcuts, and F5 refreshes the capture. Copy, save or pin closes the capture page. The capture editor supports drawing, text, numbered badges, spotlight and blur with adjustable intensity; the rectangle tool switches between rectangle and ellipse, and the arrow tool switches between straight and polyline with optional start/end heads (the non-head end is rounded); it also offers rectangle corner radius, text size/background/outline/alignment, copy, save, and pin. There is no separate select tool: click any existing object to select and move it even while a drawing tool is active; hold Shift to draw squares/circles, resize with handles, copy, paste, delete, recolor or resize its stroke, and double-click text to edit it directly on the image. Pin annotations and scroll-capture results also edit in place without a standalone editor; pin annotations keep crop, rotate 90° and flip in their More menu, while scroll results offer crop only; scroll results support Ctrl/Cmd+wheel zoom, Space+left-drag pan, Ctrl/Cmd+0 to fit, and Ctrl/Cmd+1 for 100%, while screenshot and pin editing keep their native size. In crop mode, drag a region and press Enter to apply or Esc to cancel; rotate/flip flattens annotations and can be undone.
- Window captures always use a system-style shadow and padding (24 by default; 24/48/96 available) with a transparent, system, light or dark background; there is no border, shadow toggle or shadow color. Region captures can enable corner radius and shadow from the Effects menu in the selection info bar. Choose annotation colors in the editor toolbar; Settings → Capture → Annotations only controls whether all tools share one color. The screenshot filename template lives under Settings → General and defaults to `Capin_{date}_{time}` with automatic numeric suffixes on conflict; edits save when the field loses focus or Enter is pressed.
- Pin windows can stay on top, change opacity and zoom, and rotate image pins clockwise by 90 degrees; no action bar is shown by default. Drag to move the pin, hold Alt to drag from anywhere, or drag over text to select it; press Esc or middle-click to close a pin; right-click opens a native light/dark menu with rotate, edit, copy, save and close (plus click-through). After click-through, use Release pinned windows in the tray menu; pins restore after a restart. Press the pin hotkey again to cycle through the last 7 days of final screenshot exports (up to 50).
- Image pins can recognize text and QR codes according to Settings → Capture → Pins: hold Shift and drag over text to select it, then use the system copy shortcut; double-click to select a word; drag anywhere else to move the pin.
- Recording saves MP4 with the system H.264 encoder at a fixed 30 fps and automatic bitrate, and includes the cursor by default (disable it in Settings → Recording). The demo cursor offers light yellow, light blue, light purple and light green presets plus a custom color. A 3-second countdown starts the capture. Select a region and click the record button in the toolbar; pause while recording, and stop saves one video file.
- Captures and recordings stay local; no upload, no telemetry.

## Build from source

Requires a stable Rust toolchain. Cross-platform core crates build and test on any supported host; Windows/macOS platform layers and installers are built by CI on the matching OS.

```sh
cargo build --release
cargo test
```

## License

Released under the MIT License. See [LICENSE](LICENSE).
