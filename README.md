<a id="chinese"></a>

<p align="center">
  <img src="assets/icon/capin_icon_1024.png" alt="Capin logo" width="160" />
</p>

# Capin

Capin 是面向 Windows 10+ 与 macOS 13+ 的截图、标注、贴图、OCR、长截图与录屏工具。一个快捷键截取区域、窗口或元素，原地标注、贴图置顶、复制文字、拼接长页或录制 MP4，简单、高效、类原生。

支持 Windows 10+（x64 / arm64）与 macOS 13+（Apple Silicon / Intel）。窗口截图保留系统风格阴影与内边距；Windows 与 macOS 14+ 补主题描边（1px / 1pt），macOS 13 保留原生窗口效果。区域截图可单独设置圆角与阴影。Windows 11 上托盘空闲约 5 MB，截图期间约 100–150 MB，完成后逐步回落。所有内容仅在本机处理，不上传、无遥测。

[English](#english) | 中文

## 安装

**Windows** — 从 [Releases 页面](https://github.com/fynhq/capin/releases) 下载 `Capin_Windows_<arch>_v<version>_setup.exe` 并运行。安装器按当前用户安装，无需管理员权限；绿色包 `..._<arch>_...zip` 解压后直接运行其中的 `Capin.exe`。构建未签名；如出现 SmartScreen，选择“更多信息”→“仍要运行”。

**macOS** — 下载 dmg，把 `Capin.app` 拖到 `/Applications`（或 `~/Applications`），启动 `.app`，不要直接运行 `Contents/MacOS` 内的二进制。构建未签名、未公证；如被系统拦截，移除隔离标记：

```sh
xattr -d -r com.apple.quarantine /Applications/Capin.app
```

元素识别需在“系统设置 → 隐私与安全性 → 辅助功能”中授权 Capin。每次安装新版本后，先在终端清除旧授权，再在 Capin 的“设置 → 通用 → 权限”中重新授权（屏幕录制授权后需重启 Capin）：

```sh
tccutil reset Accessibility io.github.fynhq.capin
tccutil reset ScreenCapture io.github.fynhq.capin
```

**升级与卸载** — 配置保留在用户数据目录，截图与录屏保存在图片/视频目录，需手动删除。没有自动更新：Windows 运行新的 setup 覆盖安装（绿色包替换目录），macOS 替换 app 后重新移除隔离标记并重新授权。卸载时，Windows 在“设置 → 应用”中卸载（绿色包直接删除目录），macOS 删除应用。每个 Release 附带 `SHA256SUMS`；可在包目录执行 `sha256sum -c SHA256SUMS`（macOS 用 `shasum -a 256 -c SHA256SUMS`）。

## 使用

- **开始截图** — 单击托盘图标、托盘菜单或全局热键。悬停选窗口/元素，滚轮切换层级，拖拽选区域，桌面空白单击截整屏。按住 Ctrl 进入窗口模式，单击直接标注；Esc、Cmd/Ctrl+W/Q 或再次按截图快捷键取消，右键确认标注、不退出。
- **选区与标注** — 按 `R` 切换自由/1:1/4:3/16:9；放大镜显示像素颜色与尺寸（`Shift` 切换十六进制/RGB，`C` 复制色值）。在 overlay 内直接标注：矩形/椭圆、箭头、荧光笔、文字、模糊、序号与聚光灯；区域截图标注后可拖选区边/角手柄调整导出范围，框外标注保留但隐藏。Enter 复制；空格隐藏工具栏，F1 显示快捷键，F5 只刷新底图。
- **编辑器** — `Ctrl/Cmd+C/S` 复制/保存；对象可移动、缩放、复制粘贴、删除与改样式；右键或 Esc 退出当前工具。贴图支持裁剪、旋转、翻转；长截图支持裁剪，并用 `Ctrl/Cmd+滚轮` 缩放、`Space+拖动` 平移、`Ctrl/Cmd+0/1` 适应/100%。
- **贴图** — 置顶、透明度、平滑缩放、旋转、多贴、穿透、重启恢复、标注与自动 OCR；空白或边缘拖动，Alt 任意位置拖动；Esc 或中键关闭；未划选文字时 `Ctrl/Cmd+C/V/S` 复制整图、粘贴为新贴图、保存图片；F1 显示快捷键；右键菜单含旋转、标注、穿透、设置与关闭；二维码悬停可打开或复制；按贴图键沿最近 7 天截图与剪贴板图片的同一时间线回看。
- **录屏** — 框选或悬停窗口后点工具栏录屏；3 秒倒计时，F9 暂停/继续，F10 结束；MP4（系统 H.264，固定 30 fps），无音频与摄像头；始终录入光标，演示高亮可在设置开启；停止后保存单个 MP4，可打开文件夹。
- **设置** — 保存目录、文件名模板、热键、开机自启、语言、标注统一颜色、贴图 OCR、录屏演示光标与高亮颜色均在设置页；标注颜色与线宽在编辑器工具栏选择。数据仅在本机处理，无遥测、云服务或账号。

## 从源码构建

要求 stable Rust 工具链（1.99+；CI 固定 1.100.0）。`cargo build` 与 `cargo test` 覆盖跨平台 core：

```sh
cargo build --release
cargo test
```

Windows/macOS 平台层与官方安装包由 CI 在对应系统构建。

## 许可

基于 MIT 许可发布，见 [LICENSE](LICENSE)。

---

<a id="english"></a>

# Capin

Capin is a screenshot, annotation, pin, OCR, scrolling-capture, and screen-recording tool for Windows 10+ and macOS 13+. One hotkey captures a region, window, or element; annotate it in place, pin the result, copy recognized text, stitch long pages, or record an MP4 — simple, fast, native.

It runs on Windows 10+ (x64, arm64) and macOS 13+ (Apple Silicon, Intel). Window captures keep the system-style shadow and padding; Windows and macOS 14+ add a theme border (1px / 1pt), while macOS 13 keeps the native window treatment. Region captures can set their own corner radius and shadow. The tray app idles at about 5 MB on Windows 11 (release build), peaks at roughly 100–150 MB during a capture, and settles back down afterwards. Everything is processed locally — no upload, no telemetry.

English | [中文](#chinese)

## Install

**Windows** — Download `Capin_Windows_<arch>_v<version>_setup.exe` from [Releases](https://github.com/fynhq/capin/releases) and run it. Setup installs per-user and needs no administrator rights; the portable `..._<arch>_...zip` extracts to a folder, run `Capin.exe` from inside it. The build is unsigned: if SmartScreen appears, choose **More info → Run anyway**.

**macOS** — Download the dmg, drag `Capin.app` to `/Applications` (or `~/Applications`), and launch the app, not the binaries inside `Contents/MacOS`. The build is unsigned without notarization; if macOS blocks it, remove the quarantine flag:

```sh
xattr -d -r com.apple.quarantine /Applications/Capin.app
```

Element detection needs Accessibility permission under **System Settings → Privacy & Security**. After each new version is installed, clear the old grants in Terminal, then re-authorize in Capin under Settings → General → Permissions (restart Capin after granting Screen Recording):

```sh
tccutil reset Accessibility io.github.fynhq.capin
tccutil reset ScreenCapture io.github.fynhq.capin
```

**Upgrade and uninstall** — Settings stay in the user data directory; captures and recordings go to the Pictures and Videos folders until removed manually. There is no auto-update: on Windows run the new setup over the existing install (or replace the portable folder); on macOS replace the app, remove the quarantine flag again, and re-authorize. To uninstall, use Settings → Apps on Windows (or delete the portable folder) and delete the app on macOS. Each release includes `SHA256SUMS`; verify with `sha256sum -c SHA256SUMS` (on macOS use `shasum -a 256 -c SHA256SUMS`).

## Usage

- **Start a capture** — click the tray icon, use the menu, or press the global hotkey. Hover to pick a window or element, scroll to change level, drag a region, or click empty desktop for the full display. Hold Ctrl for window-only mode and click to annotate; Esc, Cmd/Ctrl+W/Q, or the capture hotkey again cancels; right-click confirms an annotation without exiting.
- **Selection and annotation** — press `R` to cycle free / 1:1 / 4:3 / 16:9; the magnifier shows pixel color and size (`Shift` switches hex/RGB, `C` copies the value). Annotate in the overlay with rectangle/ellipse, arrows, highlighter, text, blur, numbered badges, or spotlight; region captures can resize the export frame afterwards and keep outside objects hidden until it grows. Enter copies; Space hides the toolbar, F1 shows shortcuts, F5 refreshes only the base image.
- **Editor** — `Ctrl/Cmd+C/S` copies/saves; objects can be moved, resized, copied/pasted, deleted and restyled; right-click or Esc leaves the current tool. Pins support crop/rotate/flip; scrolling captures support crop and `Ctrl/Cmd+wheel` zoom, `Space+drag` pan, `Ctrl/Cmd+0/1` fit/100%.
- **Pins** — always on top, opacity, smooth zoom, rotation, multiple pins, click-through, restart restore, annotation and automatic OCR; drag from blank areas or edges, Alt+drag anywhere; Esc or middle-click closes; `Ctrl/Cmd+C/V/S` copies, pastes a new pin, and saves the whole pin when no text is selected; F1 shows shortcuts; the right-click menu offers rotate, edit, click-through, settings and close; hovering a recognized QR code opens or copies it; the pin hotkey walks one timeline of screenshots and copied images from the last 7 days.
- **Recording** — select a region or hover a window and click record; 3-second countdown, F9 pause/resume, F10 stop; MP4 via the system H.264 encoder at 30 fps, no audio or camera; the cursor is always captured and the demo highlight is configurable; stop saves one MP4 and can open its folder.
- **Settings** — save folders, filename template, hotkeys, launch at login, language, the annotation uniform-color toggle, pin OCR, and the recording demo cursor/highlight color live in Settings; annotation colors and widths are chosen in the editor toolbar. All data stays local; there is no telemetry, cloud service, or account.

## Build from source

Requires a stable Rust toolchain (1.99+; CI pins 1.100.0). `cargo build` and `cargo test` cover the cross-platform core:

```sh
cargo build --release
cargo test
```

Windows/macOS platform layers and official packages are built by CI on the matching OS.

## License

Released under the MIT License. See [LICENSE](LICENSE).
