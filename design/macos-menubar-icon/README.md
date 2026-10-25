# macOS 菜单栏单色图标提案

## 设计方向

**北极星：在 18 pt 的菜单栏里，用安静的单色轮廓保留 Image Feedback 的“图片＋意见”身份。**

沿用彩色应用图标的图片框、山形折线、反馈气泡尾部和右上标记；为小尺寸重新绘制，不直接把 512 px Logo 缩小。移除靛蓝底板和珊瑚红，使用透明背景和单色 alpha 轮廓；外框在右上留出缺口，避免小尺寸的标记与边框糊在一起。

- SVG 源稿：[menubar-icon.svg](../../assets/menubar-icon.svg)，22×22 几何网格。
- 1× PNG：[18×18](../../assets/menubar-icon.png)。
- Retina PNG：[36×36](../../assets/menubar-icon@2x.png)。
- [明暗效果预览](preview.png)：上方为 18 pt / 2× 菜单栏上下文，下方为图形放大检查。**这是设计合成预览，不是 macOS 实机截图。**

## 接入边界

本轮只制作资源和预览，等待用户确认，不修改菜单栏加载逻辑、不替换已发布包。确认后仅 macOS resident 使用该资源并开启 `TrayIconBuilder::with_icon_as_template(true)`；由 AppKit 根据菜单栏背景及选中态自动着色，而不是根据应用深浅主题硬编码白色。现有 tray-icon 0.21.3 会将菜单栏图片设为 18 pt 高，可输入 36 px PNG 保留 Retina 清晰度。

Dock、Applications、Windows EXE 及 Windows 托盘继续使用现有彩色图标。

## 检查与复现

```sh
python scripts/generate-menubar-icon.py --preview design/macos-menubar-icon/preview.png
```

开发期需要 Pillow；程序运行不依赖 Python/Pillow。几何读取 SVG，复用现有图标生成器的路径解析，预览图也由同一份几何生成。

已检查输出尺寸、透明四角、非空 alpha 和纯黑 RGB；已查看明暗预览中的菜单栏相邻图标及放大轮廓。局部低风险设计由主代理自检，未作独立视觉评审；尚未在真实 Mac 菜单栏验证，也未声称系统明暗切换已接通。本轮不涉及动效或界面文案修改。
