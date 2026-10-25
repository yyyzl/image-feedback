# 第三方许可与来源

## 应用与智能选区

应用原创代码使用[MIT许可](<LICENSE>)。纯Rust[智能选区引擎](<src/smart_selection.rs>)与[合成样例生成器](<scripts/generate-smart-fixtures.mjs>)根据[yyyzl/dsh-image-feedback](https://github.com/yyyzl/dsh-image-feedback)固定提交 `6f256a67ecfc043dacd2614382d9c19f2437f66d` 的MIT实现移植，保留 Copyright (c) 2026 Image Feedback contributors。

本项目没有复制EditHere的C++代码、图标或设计资产。[内置合成示例](<assets/demo.png>)由[本项目脚本](<scripts/create-demo.py>)生成；应用图标与界面素材随应用代码许可发布。不打包操作系统字体。

## Slint不由根目录MIT覆盖

Slint 1.18.1包声明：`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`。桌面发行选择适用的[Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0](<licenses/slint-royalty-free-2.0.md>)，应用“··· → 关于图片反馈”保留官方 `AboutSlint` attribution；macOS原生窗体也保留这个入口。

Slint及其商标/组件不因应用根目录标记MIT而变成MIT。项目持有人应确认许可适用性、完整分发义务和将来版本变化；此说明不是法律意见。

## 依赖通知随包提供

版本由[Cargo.lock](<Cargo.lock>)固定。image、serde、serde_json、base64、rfd、arboard、zip、tray-icon、winit、winresource及其传递依赖保留各自许可。

[许可收集脚本](<scripts/collect-licenses.py>)按目标实际normal/build依赖生成完整文本HTML与JSON清单；Windows安装目录和macOS应用Resources中均附带。少数crate未包含workspace许可，原文按[固定来源及SHA-256](<packaging/license-overrides.json>)存于[第三方许可目录](<licenses/third-party/>)；新增依赖缺失原文时打包失败，不能仅用包名清单替代。

`dispatch 0.2.0`仅在上游Cargo元数据中声明MIT，没有单独许可文件；因此附[透明说明](<licenses/third-party/dispatch-0.2.0/NOTICE.md>)及未改写占位符的SPDX标准MIT模板，不擅自编造版权年份或权利人。

Windows构建静态链接CRT，不从开发机复制System32 DLL。操作系统库、SDK及构建工具仍受其各自条款约束。Inno Setup仅用于生成安装器，其编译工具不作为应用运行组件交付。完整依赖清单不能替代维护者的许可审查。
