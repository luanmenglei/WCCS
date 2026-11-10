# 路线图与交接

更新日期为 2026-11-10。

## 当前状态

M0 已完成仓库初始化。M1 已交付只读采集器 `wccs-inventory` 和 Rust 原生用户会话等待界面 `wccs-session`。等待界面已通过开发机身份展示、输入提示、重复启动、正常退出及异常重启验证。M2 首个工具配置闭环已提供 Rust 服务、React 管理 UI 和 Windows 检查入口，实际 Linux 运行待补。Windows 服务安装与批量控制按后续切片实施，机房镜像、多屏、系统会话和登录复原验证继续补齐。

采集器当前为 0.1.2，工具检查依据完整路径配置执行。候选清单已提供，路径由服务端统一维护的职责已写入需求和 ADR 0004，本地 CLI 通过 `--tools-config` 接收配置。M2 已接入管理页面、持久化、版本化下发及结果回传，见 [任务记录](tasks/m2-tool-configuration.md)。

当前方向已按用户要求调整为 Linux 服务端和原生 Windows 客户端，主实现语言选择 Rust，管理界面使用 React + TypeScript + Vite，见 [ADR 0002](decisions/0002-linux-rust-stack.md)。

选手侧 compilecpp、submit 采用独立脚本包，按维护成本选择复用或自行实现，由 WCCS 统一配置、下发、激活和回退，见 [ADR 0003](decisions/0003-contest-script-integration.md)。

开发机工具版本沿用下方更新记录。M1 已固定 Rust 1.99.1、x86_64-pc-windows-msvc、MSVC 工具集 14.44.35207 和 Windows SDK 10.0.19041.0，实际链接器为 14.44.35222.0。本机也安装了 SDK 10.0.26100.0，验收脚本显式选用 10.0.19041.0。Rust 与 C 最小程序、采集 API、发布程序均在开发机运行通过。机房 Windows 10 和 Linux 环境验收仍待实施。

## 阶段与完成条件

| 阶段 | 范围 | 完成条件 | 状态 |
| --- | --- | --- | --- |
| M0 | 仓库、项目指令、约束、架构与流程 | 检查及回归测试通过，形成可继续开发的本地提交 | 已完成 |
| M1 | 环境采集与 Windows 原型 | 只读环境报告、Rust 原生等待界面、登录与复原验证记录，确定 Rust/MSVC/Windows SDK 工具链 | 进行中，采集与等待界面开发机切片完成，现场验证待补 |
| M2 | Linux 服务与管理 UI 闭环 | Axum 服务、设备注册与心跳；浏览器登录、工具路径配置与检查回传、座位图、名单导入与分配、机器详情、批量等待界面控制、任务反馈、角色与审计 | 进行中，首个工具配置切片 Windows 闭环通过，Linux 实测待补 |
| M3 | 比赛流程与脚本工具 | 教学离线、定时安排、断线恢复；compilecpp / submit 脚本、离线依赖与 HTTPS 发布；比赛控制、脚本配置、DOMjudge 核对及换机页面联调 | 待开始 |
| M4 | 文件分发与 P2P 管理页面 | 分块下载、校验、发布、续传、互传与回源；上传、选择目标、发布安排、逐机进度和补发页面 | 待开始 |
| M5 | 设备策略与赛后恢复页面 | USB/关机策略、告警处理；归档、清理、重启、教学恢复页面及测试机验证 | 待开始 |
| M6 | 容量与现场演练 | 500 模拟客户端压测、300 座位 UI 更新、多管理员浏览器流程、Linux 部署及真实机房演练 | 待开始 |

阶段描述依赖关系与验收范围，具体实施按可运行切片推进。信息中心待确认项影响对应功能，状态逻辑和模拟适配器可继续开发。

## M2 工具配置切片交接

2026-11-10 在 `feat/tool-configuration` 完成服务、管理页面与 Windows 检查入口。核对基线 `8da0e73`、`951fc24`，开始时工作区干净。具体启动、浏览器路径、依赖版本及证据见 [任务记录](tasks/m2-tool-configuration.md) 和 [ADR 0005](decisions/0005-m2-tool-configuration-slice.md)。

- PostgreSQL 持久化配置、目标任务、结果、请求去重与审计。浏览器登录、候选选择、自定义工具、完整路径编辑、发布和重新检查已接入。
- Windows 入口复用采集器文件检查与版本读取，携带独立设备身份、配置版本、任务编号和结果。持久化 pending 回传、重复请求、并发版本冲突和旧结果处理均有回归。
- 本切片采用一份全局配置与目标集合、单管理员权限、HTTP(S) 轮询、普通进程客户端。WSS、Windows 服务与等待界面控制、设备凭据轮换及角色管理后续接入。
- 实际 Windows + PostgreSQL + 浏览器联调通过。36 项 Rust 常规测试、1 项数据库 HTTP 生命周期测试、5 项前端测试及构建/格式/Clippy 检查通过。Linux GNU 目标交叉类型检查通过。
- 本机 WSL 未安装，未发现 Docker/Podman 或已配置远端。Linux 原生构建和服务运行、HTTPS 跨机闭环仍缺具体环境。

本地实现提交为 `416bc1f`（Rust 闭环）与 `d8631ac`（管理页面和验证入口），文档随当前交接提交保存。

下一项先补齐 Linux 运行与 HTTPS 联调，再开展设备凭据生命周期、WSS、Windows 服务和等待界面鉴权 IPC。机房镜像、网络与 M1 现场事项继续独立验证。

## 管理界面实施顺序

1. M2 建立 React + TypeScript + Vite 工程、页面导航、登录、权限和 API/WSS 访问层，接入 Linux 服务提供的静态资源。
2. M2 完成工具候选列表、路径配置及版本管理、配置下发和逐机检查反馈，再完成机房座位图、选手与 IP 分配、机器详情及任务中心，贯通浏览器发起操作到客户端结果返回。
3. M3 完成比赛安排、暂停和结束控制、竞赛脚本版本及配置、DOMjudge 核对及故障换机页面。
4. M4 完成资料上传、分发、发布与 P2P 进度页面。
5. M5 完成设备告警处置、代码收集和清理重启页面。
6. M6 在实际 Linux 服务上完成管理员浏览器全流程、权限和持续状态更新验收。

每项管理功能同时验收页面、API、权限及执行反馈。具体页面范围见架构文档，浏览器场景见验证方案。

## M3 竞赛脚本切片

1. 固定 compilecpp、submit 的参数、交互、退出码和输出约定，评估 WF / DOMjudge 脚本复用与自行实现的成本。核实 Q-09，记录实际采用的接口语义和来源。
2. 实现或适配编译脚本，固定编译器配置与输出规则，提供 Windows 终端可直接调用的命令入口，验证参数、中文空格路径及编译失败行为。
3. 实现或集成 Python submit 脚本，验证目标 DOMjudge 的 API 认证、比赛题目语言选择、确认交互、提交编号和网络异常。
4. 固定解释器与依赖，交付离线脚本包、配置样例、使用说明和所复用代码的许可及修改记录。
5. 通过 HTTPS 实现版本化发布、校验、激活和回退，管理页面显示各机器脚本与配置版本以及失败原因。M4 复用发布语义并接入 P2P 传输。
6. 在普通用户环境中验证实际编译、提交、更新中断、运行中更新、版本回退及复原后的重新配置。

脚本任务以普通用户调用、可独立更新和可核对结果为完成条件。

## 竞赛脚本方向更新

2026-11-10 根据用户澄清，将选手命令明确为便于统一下发和修改的脚本包，允许复用上游或自行实现。已核对 WF 2024 编译脚本说明和 DOMjudge Python submit 源码；具体脚本语言及版本由 M3 实测固定。

本次文档变更通过 `python scripts/check_repo.py`（38 个文件）、`python -m unittest discover -s scripts/tests -v`（14 项通过）和 `git diff --check`。验证范围为文档一致性与仓库工具，脚本运行和发布验证纳入 M3。

## M1 工具路径职责修正

2026-11-10 用户明确工具路径由服务端统一维护。此前实现把“采集开发工具信息”自行具体化为 PATH / 常见目录搜索，脚本适配又增加了后缀枚举。当前已移除这些搜索逻辑和 `--tool-dir`，通过配置快照明确输入边界。[ADR 0004](decisions/0004-server-managed-tool-paths.md)

- `config/tool-catalog.json` 提供候选工具和 null 路径，支持管理员选取并扩展自定义工具。
- `--list-tools` 展示候选清单，`--tools-config` 读取配置版本及每个工具的完整文件路径，未配置项报告 `not_configured`。
- 配置路径的缺失、访问拒绝和目录对象分别报告，种类由配置明确指定。Schema 3 记录配置版本及逐项结果，历史 Schema 1 / 2 报告可继续读取。
- M2 增加服务端路径管理页面、鉴权、持久化、下发和结果关联验收。当前实现覆盖 M1 本地配置检查。

验证通过 `pwsh -NoProfile -File scripts/check-rust.ps1`，包括最小程序运行、release 构建、格式检查、Clippy 和 23 项 Rust 测试。`python scripts/check_repo.py` 的 40 个文件、14 项 Python 回归及工作区 diff 检查通过。测试包含零配置文件访问、配置路径缺失时同名 PATH 文件的隔离、自定义 ID / 文件名、未配置、目录与权限错误、配置格式拒绝和历史报告兼容。

发布程序已使用显式开发配置实际运行，结果位于忽略目录 `local/environment/report-1794284335014893000-61012/`，配置位于 `local/tool-path-verification.json`。该次明确指定采集器 exe 路径并保留 submit 路径为 null，报告正确回传 `development-check-v1`、一个文件存在结果和一个未配置结果。候选清单输出校验通过，共 17 项初始未配置工具。

## M1 采集器脚本适配历史

下列 0.1.1 记录描述当时的文件发现实现，当前检查方式已按上节的路径配置责任修正。

2026-11-10 按 ADR 0003 复查既有实现，发现竞赛命令仅匹配 `.exe`。采集器已更新到 0.1.1，补充 compilecpp / submit 的 `.cmd`、`.bat`、`.ps1`、`.py` 和无扩展名入口，保留 `.exe` 启动器识别，并补充 python3、py、powershell、pwsh 文件探测。

脚本 PE 版本明确标为 `not_applicable`，摘要显示“不适用（脚本文件）”并从失败字段计数中排除。为区分新增枚举语义，输出 Schema 升为 2，当前读取模型兼容 Schema 1 合成报告。脚本语言选择、发布清单中的包及配置版本、解释器依赖和实际调用验证仍依据 M3 计划实施。

`pwsh -NoProfile -File scripts/check-rust.ps1` 的构建、格式、Clippy 和 20 项 Rust 测试通过。集成测试使用中文空格临时目录，验证两项命令的十个脚本文件均能发现、内容保持一致、脚本执行标记文件未出现。`python scripts/check_repo.py` 检查 38 个文件通过，14 项 Python 测试与 `git diff --check` 通过。说明见 [采集工具文档](inventory.md)。

## 下一项工作

建议推进 M2 的 Linux 服务、设备注册及心跳与管理 UI 基础闭环，同时安排 M1 的机房现场验证。

- 明确 Q-06 的 Linux 环境，固定服务端和前端工程依赖，接入真实构建运行检查。
- 首先交付工具候选列表、完整路径配置、鉴权、版本化下发和逐机检查反馈页面，再扩展座位分配与等待界面批量控制。
- 使用双屏、混合 DPI 设备和独立测试会话，补齐等待界面的热插拔、缩放变化、锁定／解锁、用户切换及 RDP 验证。
- 在机房 Windows 10 x64 镜像安装或核对 VC++ x64 运行库，分别用普通账户、未提升管理员和提升管理员运行现有采集器，补齐 Q-01、Q-02、Q-08。
- 登录方式与复原验证需要信息中心提供测试账户、登录策略、复原产品和可回滚镜像。D 盘重解析点另用明确测试卷验证。
- Q-04 需要真实 VLAN / ACL、服务器地址及 P2P 放行端口。Q-06 需要 Linux 发行版、CPU 架构和可运行环境，随后开展服务端构建运行验收。

等待界面通过服务与鉴权 IPC 连接服务器形成 M2 闭环。登录界面即时登录按独立原型推进，首个管理端切片固定 Node.js 及前端依赖版本。

## M1 等待界面切片验证记录

2026-11-10 在 `feat/session-waiting` 实施。先保留并核对竞赛脚本化、服务端路径管理及采集器适配，23 项 Rust 测试、14 项 Python 测试和仓库／暂存区检查通过，形成独立本地提交 `8da0e73`。

- `wccs-session` 0.1.0 使用现有固定工具链和依赖，Win32 原生控件展示合成姓名、学校、座位与等待状态。每屏一个普通窗口，键鼠消息聚合计数，冷却为 3 秒。
- 使用当前会话命名互斥量处理重复启动，WTS 状态处理锁定及断开，显示器或 DPI 变化重建窗口。异常终止后使用相同记录目录再次启动，显示恢复提示。FFI 及句柄所有权集中封装。
- 发布程序在开发机单屏 144 DPI 运行，真实窗口验证身份、键鼠计数及提示、重复启动、按钮／Esc／Alt+F4／Tab+Enter 退出、强制终止后的重启和正常重启。实际操作限定于原型窗口与进程。
- `pwsh -NoProfile -File scripts/check-rust.ps1` 通过，包含 C / Rust 最小程序、两个包的发布构建、格式、Clippy 和 32 项 Rust 测试。会话程序的 9 项自动化测试覆盖状态、尺度、互斥及持久化失败路径。
- `python scripts/check_repo.py` 的 45 文件检查、14 项 Python 回归测试和工作区 diff 检查通过，提交前检查暂存快照。最终构建通过 Tab 后空格退出实测，正常结束记录显示 `clean_exit=true`、`visible=false`，原型进程已结束。
- 实际运行记录和验证副本保存在忽略目录 `local/waiting/`、`local/waiting-verification/`。发布 exe 位于 `target/x86_64-pc-windows-msvc/release/wccs-session.exe`，大小 304640 字节，依赖 VC++ x64 运行库。
- 多屏及实际 DPI 变化、Windows 10 机房镜像、系统锁定／用户切换／RDP／注销、登录与复原仍需相应设备和测试条件。自动化状态测试的证据范围已单独注明。

完整启动与退出命令、实现边界、自审及验证结果见 [等待界面任务记录](tasks/m1-session-waiting.md)。

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
