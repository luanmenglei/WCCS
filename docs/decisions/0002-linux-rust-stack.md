# ADR 0002 Linux 服务端、Rust 客户端与 Web 管理界面

日期为 2026-11-10。状态为用户已认可，版本及平台兼容性待原型验证。本决策替代 [ADR 0001](0001-project-baseline.md) 中的初始技术选型。

## 背景与依据

用户明确服务器运行于 Linux，建议服务端使用 Node.js、Rust 或 Go，Windows 客户端使用 C++ 或 Rust，并强调管理端 UI 的完整交付。项目当前处于架构阶段，可直接调整实现方案。

本次在用户给出的范围内选择两端 Rust，服务端采用 Tokio + Axum，客户端采用 windows crate 调用 Win32 API。共享协议和纯状态逻辑可以减少两端重复定义。具体性能通过资源测量和容量测试确认，权限隔离、输入验证及 Windows FFI 的正确性分别验收。

## 决定

| 部分 | 实现方向 | 交付形态 |
| --- | --- | --- |
| Linux 服务端 | Rust、Tokio、Axum | 单体服务二进制，包含管理 API、控制连接与静态资源服务 |
| Windows 服务 | Rust、windows crate、必要的异步网络组件 | 独立服务进程，执行系统操作和设备管理 |
| Windows 等待界面 | Rust、Win32 原生窗口，绘制方式由原型确定 | 独立用户会话进程，展示身份和触碰提示 |
| compilecpp / submit | Rust | 普通用户权限下的命令行程序 |
| Web 管理界面 | React + TypeScript + Vite | 独立前端工程，构建为静态资源并随 Linux 服务交付 |
| 控制协议 | HTTPS API、WSS、版本化 JSON 消息 | 重连、确认、去重、背压及状态重同步由应用协议定义 |
| 持久化 | PostgreSQL 与文件存储 | 任务和审计持久化，归档及发布文件分开管理 |
| P2P | 受控 HTTP(S) 文件块传输 | 授权节点、摘要校验、续传及服务器回源 |

Axum 提供 WebSocket 接入能力，连接恢复、命令有效期与执行确认由 WCCS 实现。[Axum WebSocket 文档](https://docs.rs/axum/latest/axum/extract/ws/index.html)

windows crate 提供 Windows API 绑定，包括窗口、同步和系统接口。服务、会话、设备策略及登录集成按模块封装，并依据机房 Windows 版本逐项验证。[Microsoft Rust for Windows 文档](https://learn.microsoft.com/en-us/windows/dev-environment/rust/rust-for-windows)

## 管理界面交付

管理端使用 React + TypeScript 构建组件，Vite 生成静态发布资源。Node.js 用于前端开发和构建，Linux 比赛部署包提供构建后的资源。React 的 TypeScript 支持及 Vite 的静态构建方式见官方文档。[React TypeScript 文档](https://react.dev/learn/typescript)、[Vite 静态部署文档](https://vite.dev/guide/static-deploy.html)

M2 交付登录、机房座位图、选手与 IP 绑定、机器详情、批量操作和任务反馈页面。M3～M5 扩展比赛安排、DOMjudge、换机、文件分发、策略和赛后恢复，M6 验收整个浏览器操作流程。各阶段均包含页面、API、权限和浏览器检查。

## 构建与平台边界

- Rust 工程使用 Cargo workspace，协议类型和纯状态逻辑形成可在 Linux、Windows 测试的共享模块。
- Windows API 依赖限定在 Windows 专用模块及目标依赖内，Linux 服务端仅依赖跨平台模块。
- Windows 客户端使用 MSVC 工具链和 Windows SDK，在 Windows 构建和实测。最低系统版本、架构和运行库交付方式在 M1 固定。
- Linux 服务端在 Linux 环境构建和运行验收，按目标发行版确认系统库与二进制兼容性。生产服务使用专用非 root 用户，通过明确目录权限管理数据。
- 首个工程固定 `rust-toolchain.toml`、Cargo.lock 和目标平台。前端固定 Node.js 版本及 package-lock.json。
- Rust 语言业务逻辑优先安全代码，Windows FFI 封装内的 unsafe 逐处记录安全前提。句柄使用 RAII 管理，回调生命周期及 panic 边界显式处理。

unsafe 对应编译器无法核实的契约，代码审查需要验证相应前提。项目在纯业务模块禁止 unsafe，在平台适配边界保留必要的受审查实现。[Rust unsafe 文档](https://doc.rust-lang.org/std/keyword.unsafe.html)

## 影响与复查条件

2026-11-10 的 M1 首个工程已固定 Rust 1.99.1 与 x86_64-pc-windows-msvc，开发验收脚本固定 MSVC 工具集 14.44.35207 和 Windows SDK 10.0.19041.0。采集器使用 windows 0.61.3、serde 1.0.228 和 serde_json 1.0.149，提交 Cargo.lock。该切片目标为 Windows 10 x64 及更新系统，开发机已运行通过，机房镜像与 VC++ 运行库兼容性待现场验收。后续原生界面和服务继续使用本 workspace，具体证据见 [M1 任务记录](../tasks/m1-inventory.md)。

原生窗口需要自行处理多显示器、DPI、文本布局、消息循环和会话退出。M1 以完整交互原型评估工作量。Credential Provider 如有实现需要，单独验证 COM 接口、系统加载和恢复路径。

服务端与客户端共享代码时保持协议版本兼容，服务端仍独立鉴权和校验。Web 前端使用生成或契约检查的 TypeScript 类型，并进行运行时输入校验。

出现系统 API 兼容性、UI 实现成本或构建环境问题时，记录具体证据并更新决策。最终验收包括 Linux 服务运行、Windows 客户端行为及管理员浏览器全流程。
