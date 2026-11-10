# 路线图与交接

更新日期为 2026-11-10。

## 当前状态

M0 已完成仓库初始化。已有需求基线、架构、ADR、开发与验证规范，仓库基础检查已通过。应用工程、Windows 客户端和服务器尚待后续切片实现。

当前方向已按用户要求调整为 Linux 服务端和原生 Windows 客户端，主实现语言选择 Rust，管理界面使用 React + TypeScript + Vite，见 [ADR 0002](decisions/0002-linux-rust-stack.md)。

开发机已检查 Git 2.54.0、Python 3.14.2、PowerShell 7.6.9。2026-11-10 已将 Node.js 更新为 26.13.0 LTS，npm 为 11.20.0；Rust stable 更新为 rustc 1.99.1、cargo 1.99.1，rustup 为 1.29.3，已安装目标为 x86_64-pc-windows-msvc。临时 Rust 工程已通过 MSVC 编译、链接及运行，Windows SDK 具体版本、项目所需 API 及 Linux 构建运行环境待验证。项目工具链版本将在首个工程中固定，机房兼容性需另行采集。

## 阶段与完成条件

| 阶段 | 范围 | 完成条件 | 状态 |
| --- | --- | --- | --- |
| M0 | 仓库、项目指令、约束、架构与流程 | 检查及回归测试通过，形成可继续开发的本地提交 | 已完成 |
| M1 | 环境采集与 Windows 原型 | 只读环境报告、Rust 原生等待界面、登录与复原验证记录，确定 Rust/MSVC/Windows SDK 工具链 | 待开始 |
| M2 | Linux 服务与管理 UI 闭环 | Axum 服务、设备注册与心跳；浏览器登录、座位图、名单导入与分配、机器详情、批量等待界面控制、任务反馈、角色与审计 | 待开始 |
| M3 | 比赛流程与管理页面 | 教学离线、定时安排、断线恢复、compilecpp、submit；比赛控制、DOMjudge 配置核对及换机页面联调 | 待开始 |
| M4 | 文件分发与 P2P 管理页面 | 分块下载、校验、发布、续传、互传与回源；上传、选择目标、发布安排、逐机进度和补发页面 | 待开始 |
| M5 | 设备策略与赛后恢复页面 | USB/关机策略、告警处理；归档、清理、重启、教学恢复页面及测试机验证 | 待开始 |
| M6 | 容量与现场演练 | 500 模拟客户端压测、300 座位 UI 更新、多管理员浏览器流程、Linux 部署及真实机房演练 | 待开始 |

阶段描述依赖关系与验收范围，具体实施按可运行切片推进。信息中心待确认项影响对应功能，状态逻辑和模拟适配器可继续开发。

## 管理界面实施顺序

1. M2 建立 React + TypeScript + Vite 工程、页面导航、登录、权限和 API/WSS 访问层，接入 Linux 服务提供的静态资源。
2. M2 完成机房座位图、选手与 IP 分配、机器详情及任务中心，贯通浏览器发起操作到客户端结果返回。
3. M3 完成比赛安排、暂停和结束控制、DOMjudge 核对及故障换机页面。
4. M4 完成资料上传、分发、发布与 P2P 进度页面。
5. M5 完成设备告警处置、代码收集和清理重启页面。
6. M6 在实际 Linux 服务上完成管理员浏览器全流程、权限和持续状态更新验收。

每项管理功能同时验收页面、API、权限及执行反馈。具体页面范围见架构文档，浏览器场景见验证方案。

## 下一项工作

执行 M1 的只读环境采集切片。

- 采集 Windows 版本、版本号、架构、当前账户类型与权限、卷身份、基础网络和已安装工具。
- 真实机器标识与网络信息导出到忽略目录，对外展示脱敏结果。
- 工具运行过程只读，使用合成数据覆盖缺失字段与权限不足。
- 生成 Q-01、Q-02、Q-08 的已有证据与现场待补项。
- 依据目标系统证据固定 Rust 工具链、MSVC 目标和 Windows SDK，建立第一个最小可运行工程。
- 验证 Linux 构建运行条件并收集 Q-06；首个管理端切片固定 Node.js 及前端依赖版本。

此后优先验证用户会话等待界面，再连接服务器形成 M2 闭环。登录界面即时登录按独立原型推进。

## 开发机工具链更新记录

2026-11-10 按 Node.js 最新 LTS 与 Rust stable 更新本机工具链。版本来源为 [Node.js 官方下载页](https://nodejs.org/en/download) 与 [Rust 官方发布记录](https://blog.rust-lang.org/releases/)。

- `winget upgrade --id OpenJS.NodeJS.LTS --exact --source winget --silent --accept-source-agreements --accept-package-agreements --disable-interactivity` 成功，Node.js 从 24.13.0 升级至 26.13.0，随附 npm 从 11.6.2 升级至 11.20.0。
- `rustup update stable` 成功，rustc 和 cargo 从 1.93.0 升级至 1.99.1，rustup 从 1.28.2 自更新至 1.29.3。rustfmt、Clippy、rust-src、标准库与文档组件均已安装。
- `node --version`、`npm --version`、`rustc --version`、`cargo --version`、`rustup --version` 与 `rustup target list --installed` 核对通过。Node.js ESM、内置 assert 与 crypto 模块冒烟测试通过。
- 在系统临时目录通过 `cargo new --bin --name wccs_toolchain_smoke --vcs none <临时目录>` 创建无依赖工程，`cargo fmt --all -- --check`、`cargo clippy --offline --all-targets -- -D warnings`、`cargo test --offline`、`cargo run --offline` 均通过。测试入口执行成功，模板包含 0 项单元测试；程序输出 `Hello, world!`。
- `python scripts/check_repo.py` 通过，覆盖 18 个文件；`python -m unittest discover -s scripts/tests -v` 的 14 项测试通过；`git diff --check` 与 `git diff --cached --check` 通过。
- 当前证据覆盖开发机工具链与最小程序。应用工程的依赖兼容性、Linux 服务及机房 Windows 10 验收随对应切片开展。

## 架构调整验证记录

2026-11-10 完成 Linux / Rust 技术方向调整，并补充管理 UI 的页面范围、阶段及浏览器验收。Rust 源文件已纳入文本检查，Cargo target 产物加入忽略规则。

- `python scripts/check_repo.py` 通过，覆盖 18 个仓库文件。
- `python -m unittest discover -s scripts/tests -v` 的 14 项测试通过，新增 Rust 源文件约束验证。
- `git diff --check` 通过。
- 当前证据覆盖架构、开发约束和仓库工具；Linux 服务、Windows 客户端及管理页面的构建与运行验证在 M1～M6 实施。

## 初始化验证记录

验证日期为 2026-11-10，环境为当前 Windows 开发机。

| 命令或验证 | 结果 |
| --- | --- |
| `pwsh -NoProfile -File scripts/setup.ps1` | 通过，已设置当前仓库的 LF 与 pre-commit hook |
| `python scripts/check_repo.py` | 通过，17 个仓库文件 |
| `python scripts/check_repo.py --staged` | 通过，17 个暂存文件 |
| `python -m unittest discover -s scripts/tests -v` | 13 项通过 |
| 临时 Git 仓库真实提交演练 | hook 拒绝行末空白，暂存修复后提交成功 |
| `git diff --cached --check` | 通过 |

本地基线提交说明为 `chore(repo): initialize WCCS architecture and development workflow`。远端托管地址尚待指定，后续可将相同检查接入对应 CI。
