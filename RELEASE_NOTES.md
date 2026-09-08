轻量 DeepSeek Harness 启动器，基于 Tauri 2 + Rust。

- 紧凑主窗口，环境配置、安装管理、日志移至二级页面。
- 自动定位或手动指定 Node，复用 npm 全局 Harness 安装。
- 支持启动、停止、重启、托盘和可选自动打开浏览器。
- 提供 Windows x64 安装包，以及 macOS Apple Silicon / Intel DMG。

启动器不包含 Node 或 Harness。Windows 安装包会在缺少 WebView2 时下载该运行时。当前发布未配置 Windows 代码签名或 Apple Developer ID 签名、公证。
