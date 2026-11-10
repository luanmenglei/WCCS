# 路线图与交接

更新日期为 2026-11-10。

## 当前状态

M0 已完成仓库初始化。M1 首个只读环境采集切片已完成，Cargo workspace 中的 `wccs-inventory` 可构建运行并导出 JSON 与脱敏摘要。Windows 服务、会话界面、Linux 服务及管理 UI 按后续切片实施。

当前方向已按用户要求调整为 Linux 服务端和原生 Windows 客户端，主实现语言选择 Rust，管理界面使用 React + TypeScript + Vite，见 [ADR 0002](decisions/0002-linux-rust-stack.md)。

开发机工具版本沿用下方更新记录。M1 已固定 Rust 1.99.1、x86_64-pc-windows-msvc、MSVC 工具集 14.44.35207 和 Windows SDK 10.0.19041.0，实际链接器为 14.44.35222.0。本机也安装了 SDK 10.0.26100.0，验收脚本显式选用 10.0.19041.0。Rust 与 C 最小程序、采集 API、发布程序均在开发机运行通过。机房 Windows 10 和 Linux 环境验收仍待实施。

## 阶段与完成条件

| 阶段 | 范围 | 完成条件 | 状态 |
| --- | --- | --- | --- |
| M0 | 仓库、项目指令、约束、架构与流程 | 检查及回归测试通过，形成可继续开发的本地提交 | 已完成 |
| M1 | 环境采集与 Windows 原型 | 只读环境报告、Rust 原生等待界面、登录与复原验证记录，确定 Rust/MSVC/Windows SDK 工具链 | 进行中，采集切片完成 |
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

执行 M1 的 Rust 原生用户会话等待界面切片。

- 在现有 Cargo workspace 中增加独立用户会话程序，使用合成姓名、学校和座位号。
- 验证 Win32 窗口、输入触碰提示、多屏、DPI、会话切换及异常退出行为。
- 在机房 Windows 10 x64 镜像安装或核对 VC++ x64 运行库，分别用普通账户、未提升管理员和提升管理员运行现有采集器，补齐 Q-01、Q-02、Q-08。
- 登录方式与复原验证需要信息中心提供测试账户、登录策略、复原产品和可回滚镜像。D 盘重解析点另用明确测试卷验证。
- Q-04 需要真实 VLAN / ACL、服务器地址及 P2P 放行端口。Q-06 需要 Linux 发行版、CPU 架构和可运行环境，随后开展服务端构建运行验收。

等待界面完成后连接服务器形成 M2 闭环。登录界面即时登录按独立原型推进，首个管理端切片固定 Node.js 及前端依赖版本。

## M1 采集切片验证记录

2026-11-10 完成。架构与约束先经检查提交为 `dd6ae84`，采集实现位于 `feat/environment-inventory` 分支。

- `pwsh -NoProfile -File scripts/check-rust.ps1` 通过，包含固定工具链核对、Windows SDK C 程序及 Rust 最小程序的编译运行、release 构建、rustfmt、Clippy 和 17 项 Rust 测试。
- 发布 exe 在开发机真实运行通过，采集到 5 个本地卷、57 个网络接口，系统内核版本为 10.0.26300.9711，当前为未提升管理员令牌。报告明确标记 `development`。这些信息作为开发环境证据保存。
- 真实 JSON 与脱敏摘要位于忽略目录 `local/environment/`，最小程序产物位于 `artifacts/toolchain-smoke/`，发布程序位于 `target/x86_64-pc-windows-msvc/release/`。
- 17 项测试覆盖权限拒绝、缺失字段和文件版本、工具未发现、IPv4 字节序、未知权限序列化、脱敏、重复导出、中文空格路径、CLI 参数和真实导出、目标文件保护。
- 仓库检查与 14 项 Python 回归测试通过，工作区及暂存区 diff 检查通过。自审修复令牌提升查询的 `ERROR_BAD_LENGTH`，保留搜索错误目录和局部失败，核对 FFI 句柄释放与缓冲区生命周期。
- `dumpbin /dependents` 确认发布程序依赖 `VCRUNTIME140.dll` 和 Windows 系统 DLL。机房镜像需核对对应运行库及实际启动行为。

具体运行命令、报告字段、范围限制和现场条件见 [采集工具说明](inventory.md) 与 [任务记录](tasks/m1-inventory.md)。

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
