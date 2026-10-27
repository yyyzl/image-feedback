# Image Feedback v0.1.2

Windows 和 macOS 本地图片反馈编辑器。

## 本次更新

- 修复撤销、重做在批注输入框获得焦点时失效的问题；连续撤销删除批注后，仍可继续重做。支持 Ctrl/⌘+Z、Ctrl/⌘+Shift+Z 和 Ctrl+Y。
- 增加“新建项目”（Ctrl/⌘+N），有未保存修改时可先保存、不保存或取消。
- 增加批注栏右上角的“清空当前”：保留图片，确认后清除全部批注、文字意见和图块移动；一次撤销即可整体恢复。
- 将“适应”改为“显示全图”，缩放并居中显示完整图片。
- 精简 macOS 安装盘，只显示应用和 Applications 快捷方式。

## 下载与安装

- **Windows 10/11 x64**：下载 `ImageFeedback-0.1.2-windows-x64-setup.exe`，运行安装器。更新前请保存项目，并退出编辑窗口和托盘入口；安装器不会强制结束正在编辑的程序。
- **macOS 13+，Apple Silicon / Intel**：下载 `ImageFeedback-0.1.2-macos-universal-adhoc.dmg`，打开后将 Image Feedback.app 拖入 Applications，推出安装盘再启动。
- 使用同名 `.sha256` 文件核验下载完整性。

安装包包含应用所需运行内容，不要求用户安装 Rust、Qt、Node.js 或单独的 VC++ Redistributable。Windows 静态链接 CRT；macOS 安装包包含 arm64 和 x86_64 两种架构。

## 验证与签名说明

发行工作流检查动态库、运行 Rust 测试，并验证 Windows 实际安装、修复安装及卸载；macOS 在 Apple Silicon 和 Intel 上分别验证复制安装后的应用及窗口交互。窗口回归使用真实 Slint 窗口和场景键盘、指针事件，不等同于物理输入、真实 IME、干净虚拟机或最低系统版本认证。

Windows 安装器尚未签名；macOS 应用为 ad-hoc 签名，尚未通过 Apple 公证。macOS 首次启动可能需要在“系统设置 → 隐私与安全性”选择允许打开。

图片与项目始终在本地处理，原图保持不变。卸载不清理用户项目和应用数据；本版本不包含自动更新。
