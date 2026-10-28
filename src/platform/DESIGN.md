# 平台适配设计

## 设计目标

保持文档和导出与平台无关，在原生边界区分触控板平移和机械滚轮缩放，并保留各平台关闭保护。所有钩子仅属于本应用窗口。

## 方案选择

Windows 的 winit 0.30.13 将 WM_MOUSEWHEEL 和 WM_MOUSEHWHEEL 都转换成 LineDelta，单凭该值无法可靠识别触控板。以小于 120 的滚动量猜测设备会误判高分辨率鼠标，因此接入系统 Direct Manipulation，接收 PT_TOUCHPAD 联系点及其变换差量。机械滚轮继续使用原事件链，macOS 使用 winit 的 PixelDelta／LineDelta 区分连续手势与滚轮。

未使用 Windows 11 新增的触控板注册接口，以保留 Windows 10 运行范围；没有改动系统设置、cargo registry 或 winit 源码。

## 关键决策

- 本窗口子类处理 DM_POINTERHITTEST，仅当指针位于可交互画布内时接收触控板联系点；侧栏、输入框、弹窗和缩放控件继续由 Slint 命中处理。
- Direct Manipulation 仅启用双轴平移和惯性。系统提供的物理像素差量除以窗口 scale factor 后交给共同的画布平移入口；不改原图坐标、不进撤销历史。
- 只在手势或惯性活跃时以 16ms 更新系统手势状态，静止时停止定时器。
- COM 回调只写入 Arc/Mutex 的数值差量，不在可能的系统回调线程操作 UI；UI 定时回调应用变更。
- 窗口失焦、调整大小或显式鼠标／键盘操作时停止手势和惯性，避免恢复视图后继续漂移；退出时先解除窗口子类、回调及视口，再释放 COM apartment。持有关系用 Weak 避免窗口子类或定时器形成循环引用。
- macOS 生命周期继续添加缺失 delegate 方法，不替换 winit delegate；既有 Quit 防丢规则保留。

## 已知限制与验证边界

精准触控板由系统 Direct Manipulation 识别；把所有输入伪装为普通鼠标滚轮的旧驱动无法从相同消息恢复真实设备身份。真实硬件的滚动方向、惯性、多屏 DPI 和候选输入需专项测试，不能用初始化成功或场景事件声称已经认证。

## 依据

- [Microsoft：Precision Touchpad Input](https://learn.microsoft.com/en-us/windows/win32/input-precisiontouchpad/precision-touchpad-portal)
- [Microsoft：Direct Manipulation 输入处理](https://learn.microsoft.com/en-us/windows/win32/directmanipulation/directmanipulation-processing-input-with-directmanipulation)
- [Microsoft：SetContact](https://learn.microsoft.com/en-us/windows/win32/api/directmanipulation/nf-directmanipulation-idirectmanipulationviewport-setcontact)
- [Chromium：精准触控板的 DM_POINTERHITTEST／PT_TOUCHPAD 入口](https://github.com/chromium/chromium/blob/main/content/browser/renderer_host/direct_manipulation_helper_win.cc)
- 原生 ABI 与常量以锁定的 windows 0.62.2 生成绑定为准。

## 变更历史

- 2026-10-28：新增 Windows 精准触控板适配，保留机械滚轮缩放；记录原有 macOS 生命周期边界。
