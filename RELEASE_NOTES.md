# Harness Launcher v0.1.1

轻量 DeepSeek Harness 启动器，基于 Tauri 2 + Rust。

- 液态玻璃界面：柔和渐变、半透明面板和高光边缘；无系统边框、透明圆角窗口，窗口高度自适应。
- 托盘菜单根据服务状态显示启动、停止或重启。
- 修复 nvm 环境识别，读取环境变量和登录 Shell，默认优先已安装 dsh 的 Node。
- 设置页显示完整 Node 路径，支持多环境选择及手动指定。
- 管理页支持 GitHub 在线检查更新、下载安装包及 SHA256 校验。
- 支持启动、停止、重启、托盘和可选自动打开浏览器。
- 提供 Windows x64 安装包，以及 macOS Universal DMG（同一个包支持 Apple Silicon 和 Intel）。

启动器不包含 Node 或 Harness。Windows 安装包会在缺少 WebView2 时下载该运行时。macOS 使用 ad-hoc 签名并验证签名完整性，尚未进行 Apple Developer ID 签名与公证。Windows 尚未配置代码签名。
