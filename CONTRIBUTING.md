# Contributing

欢迎提交问题、文档与修复。请勿在公开 issue、日志或测试素材中包含私人截图、项目文件、凭据或真实聊天内容。

## 开发

使用仓库锁定的 Rust 工具链和对应原生 SDK：

```sh
cargo fmt --all -- --check
cargo test --release --locked
cargo build --release --locked
```

提交 UI、存储或平台生命周期更改时，应运行[打包及窗口回归](<docs/BUILD.md>)。测试使用合成图片与隔离剪贴板，不能强杀用户编辑器。不要改写原图或在默认复制文本中重新塞入交付目录等冗余信息。

## 发布与依赖

- 保留 Cargo.lock；更新依赖时复查许可、CRT/DLL导入和macOS系统动态库。
- Windows安装器AppId、macOS Bundle ID和用户数据目录是稳定标识，不随版本重新生成。
- macOS AppKit生命周期方法与固定winit版本配合，升级winit必须重新验证原生关闭、Quit、文件选择时的重入与托盘唤回。
- 不要提交签名证书、私钥、密码、个人项目、缓存、构建输出或本机验证日志。
- 自己的贡献应有权按项目MIT许可发布；第三方代码/素材须保留其版权与许可。
