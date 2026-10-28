# 原生平台适配

本目录连接 Slint 编辑器与系统窗口、生命周期和输入接口，供 `src/platform.rs` 按目标平台选择。文档和图像业务仍由编辑器及文档模块负责。

## 职责与依赖

- `macos.rs`：AppKit 标题栏、Quit／重开保护及编辑器激活，不替换 winit delegate。
- `windows.rs`：Direct Manipulation 精准触控板双轴平移；机械滚轮继续由 winit 分发缩放。
- 依赖锁定的 Slint／winit；Windows 使用已经锁定的 windows 0.62.2 和 windows-core 包，调用系统 COM、User32、Comctl32 接口，无额外运行时安装。

## 使用

窗口显示后，在 UI 线程等待 winit_window future 就绪，再调用 `TouchpadInput::install(&ui, &window)`，持有返回的 Rc 到事件循环结束。接口负责建立本窗口的手势视口，结束时解除窗口子类和 COM 回调。macOS 使用现有的 configure／install 生命周期入口。

```rust,ignore
let window = ui.window().winit_window().await?;
let touchpad_input = platform::TouchpadInput::install(&ui, &window)?;
drop(window); // 不额外持有原生窗口，适配器的 Rc 由外层保留。
```

## 验证

`cargo test --locked` 检查双轴差量与非有限输入；窗口 `--ui-smoke` 检查平移、滚轮缩放、恢复初始视图及控件不穿透。Windows 场景启动还会实际创建 Direct Manipulation 管理器与窗口子类。真实触控板方向、惯性和老驱动仍须硬件试用；场景事件不等于硬件 E2E。

实现取舍、系统接口依据与生命周期约束见 [DESIGN.md](DESIGN.md)。
