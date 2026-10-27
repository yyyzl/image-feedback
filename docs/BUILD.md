# 构建、打包与验证

## 工具链

仓库锁定Rust 1.95.0。Windows开发需要MSVC Build Tools、Windows SDK、LLVM的 `llvm-readobj`；macOS需要Xcode Command Line Tools。Python3用于开发期许可收集，Inno Setup仅用于编译Windows安装器。最终用户不需要这些构建工具。

```sh
cargo fmt --all -- --check
cargo test --release --locked
cargo build --release --locked
```

## Windows安装器

在Windows仓库目录运行：

```powershell
pwsh -File scripts/install-inno.ps1
pwsh -File scripts/package-windows.ps1
```

[工具引导](<../scripts/install-inno.ps1>)使用固定Inno Setup 6.7.3版本、官方SHA-256和Authenticode发布者验证，安装到忽略的本地工具目录，不改变运行用户的PATH。

[打包脚本](<../scripts/package-windows.ps1>)用显式MSVC target与 scoped `-C target-feature=+crt-static` 构建，避免把静态CRT参数传给宿主过程宏；导入白名单和VC运行库否定检查不通过就停止打包。程序安装名为 `ImageFeedback.exe`，内部Cargo二进制名称保留以兼容开发脚本。

产物位于 `dist/windows`。若LLVM或ISCC不在标准位置，可通过 `-Readobj`、`-Iscc` 传入；Python可用 `-Python` 指定。

运行真实安装、运行中阻止更新、同版本修复安装、已安装程序窗口测试、卸载与数据保留验证：

```powershell
pwsh -File scripts/verify-windows-install.ps1 -Installer dist/windows/ImageFeedback-0.1.2-windows-x64-setup.exe
```

验证脚本检测到已有正式安装时会停止，不覆盖用户安装。它只结束自己启动的、没有编辑器子进程的空闲测试托盘，不终止用户编辑窗口。安装器本身不使用强制进程结束。当前修复测试不是跨版本升级测试；所有升级必须保留[安装器AppId](<../packaging/windows/ImageFeedback.iss>)。

## macOS Universal DMG

在真正的Mac上运行；Windows不能替代macOS SDK、签名和磁盘镜像工具：

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash scripts/package-macos.sh
bash scripts/verify-macos.sh dist/macos/ImageFeedback-0.1.2-macos-universal-adhoc.dmg
```

[打包脚本](<../scripts/package-macos.sh>)分别构建arm64/x86_64并以lipo合并，生成Info.plist、icns、许可资源和Applications软链接；两个架构都检查动态库和部署目标。非系统动态库必须完整捆绑，不能通过删除检查来掩盖缺失依赖。最低构建目标为macOS13.0，CI运行系统与最低版本不是同一个测试维度。

当前只做ad-hoc完整性签名，不提供Developer ID、公证或Gatekeeper信任保证。添加正式签名需由维护者通过受保护GitHub Secrets配置Apple凭据；不要提交证书或密码。签名应在lipo和资源修改之后，未来新增Frameworks时从内向外处理。

[安装验证](<../scripts/verify-macos.sh>)挂载DMG、复制`.app`到独立Applications测试目录、推出DMG后在构建树外启动。它在本机架构实际创建菜单栏项并读取 `NSImage.isTemplate`，随后运行窗口回归，并检查正常入口的resident/editor进程；Universal包含两个slice不等于两个架构都运行，Intel验证使用独立Intel job。

## macOS原生生命周期

[平台模块](<../src/platform/macos.rs>)保留winit自己的AppKit delegate，只添加缺失的Quit/reopen方法；不替换delegate、不修改cargo registry。Quit转发到同一窗口关闭保护，文件选择等重入时保持窗口。固定winit版本，升级时必须复查该约定。

菜单栏父进程为Accessory，编辑器为Regular；用户私有Unix socket仅接受固定激活命令，用来恢复最小化窗口。只有持有编辑器锁的进程可创建/清理端点，后台线程阻塞等待，不做周期业务轮询。

## 许可与CI

[收集器](<../scripts/collect-licenses.py>)按实际目标的normal/build依赖收集包自带许可文本，附可移植JSON清单。已发布crate缺少workspace许可时使用[固定来源及哈希](<../packaging/license-overrides.json>)对应的原文；缺失新条目会阻止打包。[维护工具](<../scripts/fetch-upstream-notices.py>)可补齐上游文件，但更新后仍须人工审阅版权与来源。

[CI](<../.github/workflows/build.yml>)运行Windows、Apple Silicon和Intel验证。标签发行只在所需job成功后发布安装器、Universal DMG及SHA-256；源码、工具和测试数据不混入用户图片。

安装后的窗口回归包含 `smoke-test`、`delivery-smoke`、`dialog-smoke` 和 `ui-smoke`。新增 UI 场景检查批注收起／展开、失败草稿恢复、键盘编辑及 1000 条批注和最小窗口；行为与证据边界见 [UI 对齐说明](<UI-WEB-ALIGNMENT.md>)。

窗口回归使用真实Slint窗口及场景事件、隔离剪贴板，不是操作系统鼠标/UIA E2E。场景开始前按加载、文件工作与分析状态等待就绪，附加等待上限30秒；超时保留状态和窗口截图，不跳过失败场景。macOS检查真实decoration配置，并调用AppKit `terminate:` 验证退出保护；不把Slint内容截图称为原生traffic-light截图。真实IME、触控板、多屏、Gatekeeper、干净Windows虚拟机及最低macOS版本仍需专项用户实机验收。
