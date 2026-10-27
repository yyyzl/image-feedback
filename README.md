# Image Feedback

用图片表达修改意见：圈选、点批注、移动图块，然后复制 **原图路径＋精简修改建议**。原生 Rust + Slint 桌面应用，不依赖 Harness、WebView、Node.js 或 Qt 运行时。

[下载发行版](https://github.com/yyyzl/image-feedback/releases) · [构建与安装验证](https://github.com/yyyzl/image-feedback/actions) · [反馈问题](https://github.com/yyyzl/image-feedback/issues)

> 当前为早期开源版本。Windows 安装器尚无发布者证书；macOS DMG 为 **ad-hoc 签名、未通过 Apple 公证** 的版本，首次启动可能被系统拦截。请核验发行页 SHA-256；macOS 可通过“系统设置 → 隐私与安全性”允许本应用，不需要关闭全局 Gatekeeper。

## 安装

| 平台 | 下载内容 | 安装方式 |
|---|---|---|
| Windows 10/11 x64 | `ImageFeedback-版本-windows-x64-setup.exe` | 运行安装器，安装到当前用户的 Programs，创建开始菜单入口；可从系统应用列表卸载 |
| macOS 13+，Apple Silicon / Intel | `ImageFeedback-版本-macos-universal-adhoc.dmg` | 打开 DMG，将 **Image Feedback.app** 拖入 **Applications**，推出磁盘后启动 |

安装包包含应用运行所需的非系统内容，用户不需要安装 Rust、Qt、Node.js 或单独的 VC++ Redistributable。Windows 发行构建静态链接 CRT，并检查只导入系统 DLL；macOS 包检查只链接系统动态库。图形兼容性有问题时，Windows 开始菜单有 **Software rendering** 入口。

更新或卸载前，请先保存项目并退出编辑器及托盘入口；安装器不会强制结束编辑进程。卸载程序不会清理图片、项目或应用数据。详见[安装与数据位置](<docs/INSTALL.md>)。

## 使用

1. 导入或粘贴 PNG/JPEG/WebP/BMP 图片。
2. 选择工具下悬停查看推荐范围，单击采用，或拖动画出自己的范围。也可添加点批注、整体意见。
3. 在右侧写意见；新批注自动滚动定位并聚焦。点击标题可展开或收起，切换、失焦和收起时提交草稿；校验失败会保留文字。每条批注右上角可删除，删除可撤销。拖动已有范围可表达图块移动。
4. 点击“复制反馈说明”：仅包含一条原图路径、必要坐标约定与紧凑的编号意见。
5. 保存 `.ifp` 项目以继续编辑；需要跨设备交付时导出反馈包。

原图保持不变。磁盘来源未变化时优先给原地址；粘贴图片会自动保存 PNG。**本机路径不是公开链接**，发给其他设备或普通聊天客户端时需另附原图或反馈包。“···”菜单提供原图路径、最近反馈目录及可选 Base64 数据；接收工具需要支持解码。

右下角缩放组提供“显示全图”；左侧保留选择、点批注和平移三个工具。顶部和侧栏操作可通过 Tab 到达，图标按钮有名称与快捷键提示。底部“复制反馈说明”获得键盘焦点或悬停时会显示跨设备使用说明。

[桌面 UI 对齐说明](<docs/UI-WEB-ALIGNMENT.md>)记录批注收起、草稿安全及验证边界。

Windows 使用自绘标题栏。macOS 使用系统标题栏及左上角原生关闭、最小化和缩放按钮；更多菜单在工具栏。macOS 菜单栏使用独立的单色模板图标，由系统自动适配明暗；Dock、Applications 和 Windows 的彩色图标不变。应用消息卡片自绘，文件/目录选择器使用系统原生界面。

关闭编辑器会释放编辑器进程，独立托盘/菜单栏入口仍保留；退出该入口不会强杀尚有未保存内容的编辑器。macOS 的应用级 Quit 也经过编辑器关闭保护。

“新建项目”返回空白工作区，可重新导入或粘贴图片；有未保存修改时，可选择保存后新建、不保存或取消。原项目和图片文件不受影响。

批注栏右上角的“清空当前”保留图片，清除全部批注和文字意见，并复原图块移动；确认后执行，可按 Ctrl/⌘+Z 一次恢复。

常用快捷键：Ctrl/⌘+N 新建、Ctrl/⌘+O 导入、Ctrl/⌘+V 粘贴、Ctrl/⌘+S 保存、Ctrl/⌘+Z 撤销、Ctrl/⌘+Shift+Z 或 Ctrl+Y 重做。撤销、重做、新建和保存也适用于批注输入框；输入文字与画布编辑使用同一套项目历史。V 选择、P 点批注、H 平移、F 显示全图；空格或中键拖动平移。“显示全图”会缩放并居中，让整张图片完整显示在当前画布中。

## 范围与隐私

- 本地处理，不上传图片，无账号、遥测、自动更新、自动开机启动或网络模型调用。
- 智能推荐是图像启发式算法，不是 OCR、DOM 识别或语义模型；复杂纹理可能不推荐，始终可手动框选。
- 目前只支持图块平移，不支持旋转/缩放、EditHere 项目导入、跨重启撤销历史或崩溃恢复。
- 单图上限4000万像素、源文件64 MiB、最多1000个标记；大图仍需相应像素内存。
- 自动交付快照保留在用户应用数据目录，可自行清理；清理会使以前复制的路径失效。

## 从源码构建

需要 Rust 1.95.0 和对应平台编译工具：Windows 为 MSVC Build Tools + Windows SDK，macOS 为 Xcode Command Line Tools。开发依赖不属于最终用户运行依赖。

```sh
cargo build --release --locked
cargo test --release --locked
```

[打包、CI与验证方法](<docs/BUILD.md>)介绍 Windows 安装器、macOS Universal DMG、实际安装/卸载回归及测试边界。贡献前请阅读[贡献说明](<CONTRIBUTING.md>)。

## 许可

应用原创代码使用 [MIT](<LICENSE>)。智能选区移植自 `yyyzl/dsh-image-feedback` 的 MIT 实现，保留来源与版权。**Slint 及其他依赖保留自身许可，根目录 MIT 不覆盖它们**；发行包附带实际依赖许可与来源清单，应用“关于”保留官方 AboutSlint。见[第三方声明](<THIRD_PARTY_NOTICES.md>)。
