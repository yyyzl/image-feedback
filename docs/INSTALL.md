# 安装、更新与用户数据

## Windows

从[发行页](https://github.com/yyyzl/image-feedback/releases)下载 x64 安装器，核验 SHA-256 后运行。默认安装到当前用户的 `%LOCALAPPDATA%\Programs\ImageFeedback`，无需管理员权限。开始菜单包含正常入口和软件渲染兼容入口；桌面快捷方式可选。

安装内容包括静态 CRT 可执行文件、许可和依赖清单，不要求最终用户安装 Rust、Qt、Node.js、MSVC Build Tools 或单独 VC Redistributable。Windows 自带系统库和字体不重复打包。安装器注册卸载信息、App Paths 和 `.ifp` 的“打开方式”，不抢占 PNG/JPEG 等通用图片关联。

更新或卸载前保存项目，关闭编辑器，并从托盘菜单退出常驻入口。安装器检测运行中的应用，不调用 Restart Manager 强杀它。通过系统“已安装的应用”卸载；应用文件、注册项与快捷方式被移除，用户数据保留。

当前发行文件没有代码签名证书，SmartScreen 可能显示未知发布者；这不等于已获得微软信任或通过安全认证。

## macOS

打开 Universal DMG，将 **Image Feedback.app** 拖到 **Applications**，推出磁盘，再从 Applications 启动。应用包包含可执行文件、图标和许可，不依赖构建目录、Homebrew、Rust、Qt 或 Node。

当前构建没有 Developer ID / Apple 公证，只有 ad-hoc 完整性签名。首次下载可能被 Gatekeeper 阻止，可在“系统设置 → 隐私与安全性”针对本应用选择允许打开。不要关闭全局 Gatekeeper；发行页同时提供 SHA-256。完整签名、公证版本需要项目持有人提供 Apple Developer 凭据，不能把 ad-hoc 验签当成公证。

窗口使用原生左上角 traffic-light 控制，编辑器是正常 Dock 应用，常驻入口使用菜单栏 accessory 身份。关闭编辑器后菜单栏仍保留；退出菜单栏入口不会结束仍在编辑的进程。原生窗口关闭和应用级 Quit 都接入未保存保护。

更新时先保存并退出编辑器和菜单栏入口，再替换 Applications 中的 `.app`。卸载可将 `.app` 移到废纸篓；不会自动删除用户项目或应用数据。

## 用户数据

| 平台 | 应用数据目录 |
|---|---|
| Windows | `%LOCALAPPDATA%\image-feedback-slint` |
| macOS | `~/Library/Application Support/image-feedback-slint` |

目录名保留开发期标识，以便升级沿用已有数据。自动原图快照、编号参考图与反馈包位于其中的 `feedback-deliveries`，不自动清理；用户自行选择保存的 `.ifp` 项目在所选位置。

本机路径仅适合同机读取。发给其他电脑时另附原图/ZIP；Base64 只有在接收端支持解码时适用。不要直接修改用于既有意见定位的交付快照。
