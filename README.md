# Windows Contest Control System

WCCS 用于 Windows 10 多机房单人算法竞赛，目标规模为 200～300 台选手机。提供座位与选手绑定、比赛等待界面、统一操控、状态监测、文件分发，以及 `compilecpp`、`submit` 等竞赛命令。

日常教学时客户端离线待机。比赛时由服务器的 Web 控制台集中管理。赛后可清空 D 盘并重启，通过机房镜像复原恢复教学环境。

## 实现方向

| 部分 | 技术与部署 |
| --- | --- |
| 服务端 | Linux 上运行 Rust 单体服务，Tokio + Axum，PostgreSQL |
| Windows 客户端 | Rust 系统服务、Rust 原生会话界面及运维采集工具，通过 windows crate 调用 Win32 API |
| 选手竞赛命令 | 独立版本的脚本包，可复用上游或自行实现，统一分发 Windows 命令入口、依赖和比赛配置 |
| Web 管理界面 | React + TypeScript + Vite，静态资源随 Linux 服务端交付，管理员从浏览器访问 |
| 通信 | HTTPS API、WSS 控制与状态推送、分块文件传输及受控 P2P |

当前技术方向记录在 [ADR 0002](docs/decisions/0002-linux-rust-stack.md)。依赖版本、Windows 最低版本和 Linux 发行版在原型及部署环境确认后固定。

`compilecpp`、`submit` 的脚本交付方式见 [ADR 0003](docs/decisions/0003-contest-script-integration.md)。命令语义、复用或自行实现的选择及 Windows 运行环境在 M3 固定。

## 当前阶段

M1 已交付 `wccs-inventory` 和 `wccs-session`。采集器通过原生 Windows API 只读采集系统、账户权限、磁盘卷和网络，并按明确配置路径检查工具文件，输出 JSON 与脱敏摘要。会话原型使用 Win32 窗口展示合成身份、输入提示和退出入口，开发机交互与异常重启已验证。工具候选清单及客户端配置检查已实现，M2 已提供 Rust 服务、PostgreSQL 持久化、管理页面、设备注册心跳与配置检查回传。M2 机房座位表已支持矩形编辑、连续编号、IP 批量绑定、认证设备匹配与检查目标预览。M2 设备生命周期已提供注册窗口与审核、启用 / 禁用 / 撤销、凭据轮换和重装重新授权，客户端为 0.2.0。Windows 开发机闭环通过，实际 Linux 运行待补。机房镜像、多屏及系统会话切换仍待现场验证。

## 开发入口

需要 Git、Python 3.11 或更新版本，以及 PowerShell 7。运行以下命令安装本仓库的本地 Git hook 并验证基础设施。

```powershell
pwsh -NoProfile -File scripts/setup.ps1
python scripts/check_repo.py
python -m unittest discover -s scripts/tests -v
```

`setup.ps1` 设置本仓库的换行和 hook 配置。新克隆或新的开发环境需要运行一次。

构建并运行 Windows 采集工具需要 Rust 1.99.1、MSVC 工具集 14.44.35207 和 Windows SDK 10.0.19041.0。依赖已固定并提交 Cargo.lock。

```powershell
pwsh -NoProfile -File scripts/check-rust.ps1
& ./target/x86_64-pc-windows-msvc/release/wccs-inventory.exe --output-dir local/environment --context development
```

每次在 `local/environment/report-*/` 生成 `report.json` 和 `summary.txt`。完整 JSON 包含真实机器信息，保存在忽略目录。当前发布程序依赖 VC++ x64 运行库中的 `VCRUNTIME140.dll`。工具参数、字段状态及采集范围见 [采集工具说明](docs/inventory.md)。

`--list-tools` 输出[候选工具清单](config/tool-catalog.json)。服务端负责统一维护选中工具的实际路径，M1 本地验证使用 `--tools-config local/tools.json` 读取相同字段的版本化配置。默认运行将工具状态标为未配置。职责边界见 [ADR 0004](docs/decisions/0004-server-managed-tool-paths.md)。

## 等待界面原型

```powershell
pwsh -NoProfile -File scripts/check-rust.ps1
& './target/x86_64-pc-windows-msvc/release/wccs-session.exe'
```

每屏展示合成姓名、学校、座位号与等待状态。触碰键鼠显示 3 秒冷却提示，聚合计数保存在忽略目录 `local/waiting/`。点击“退出开发测试”、按 Esc 或 Alt+F4 可退出全部窗口，Tab 后按 Enter 也可操作退出按钮。运行条件为 Windows 10 1703+ x64，详见[任务及验证记录](docs/tasks/m1-session-waiting.md)。

## 工具配置管理闭环

```powershell
pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json
& './target/x86_64-pc-windows-msvc/release/wccs-server.exe' --config local/m2/server.json
& './target/x86_64-pc-windows-msvc/release/wccs-agent.exe' --identity local/m2/device.json
```

服务与客户端分别运行在终端。浏览器访问 `http://127.0.0.1:8787/`，登录材料位于忽略目录 `local/m2/admin-access.json`。选择候选或自定义工具，编辑完整路径、选择目标并发布，设备区显示期望/实际配置版本和逐项结果。前端生产资源位于 `web/dist/`，由服务同源提供。

上述命令使用本轮独立 PostgreSQL 测试集群。新环境初始化、Linux 原生启动、浏览器步骤及全部证据见 [M2 任务记录](docs/tasks/m2-tool-configuration.md)。工具检查使用持久操作员、角色与机房授权、一份全局配置及目标集合、HTTP(S) 轮询。普通进程入口与本轮新增的 Windows 服务宿主复用同一检查逻辑。操作员页面、全局事件、迁移及完整验证见 [操作员与事件记录](docs/tasks/m2-operator-events.md)。

机房操作从“机房与座位”进入，创建布局、应用编号规则和批量 IP 映射，预览后保存。选择整机房或部分座位可展开实际检查目标。详细步骤、代理配置与验收边界见 [机房座位表记录](docs/tasks/m2-room-seating.md)。

管理台已按 Natsume 核心流程分为座位、设备、检查任务、机房设置与工具配置，默认入口为 `#/seats`。界面规范与功能对照见 [对齐记录](docs/tasks/natsume-alignment.md)。

## 文档导航

| 文档 | 用途 |
| --- | --- |
| [AGENTS.md](AGENTS.md) | Codex 工作入口和项目约束 |
| [需求基线](docs/requirements.md) | 已确认需求、范围和现场未知项 |
| [架构](docs/architecture.md) | 组件、状态、身份、协议和部署设计 |
| [初始架构决策](docs/decisions/0001-project-baseline.md) | 架构基线与技术选型历史 |
| [Linux 与 Rust 决策](docs/decisions/0002-linux-rust-stack.md) | 当前实现语言、Web 管理界面及构建边界 |
| [竞赛脚本交付决策](docs/decisions/0003-contest-script-integration.md) | 实现选择、Windows 适配及独立更新方式 |
| [开发规范](docs/development.md) | 代码、Git、Codex 开发与交接流程 |
| [验证方案](docs/validation.md) | 自动化检查、容量目标和真机演练 |
| [采集工具说明](docs/inventory.md) | 构建、运行、报告结构及只读边界 |
| [等待界面原型](docs/tasks/m1-session-waiting.md) | 窗口启动、退出、恢复及实际验证 |
| [设备生命周期](docs/tasks/m2-device-lifecycle.md) | 注册审核、凭据更新、重装恢复及实际验证 |
| [路线图](docs/roadmap.md) | 当前状态、任务顺序和完成条件 |

## 从 Codex 开始下一项工作

可以向 Codex 提交以下任务。

> 阅读 AGENTS.md、需求、路线图与 M2 比赛账户任务记录，继续补齐独立 Windows 10 原生登录和账户恢复矩阵，再按真实机房镜像验证 Administrator、SCM / Session 0、复原及干预恢复。Linux 原生运行与现场网络按环境继续补齐。

机房真实复原、设备策略和自动登录演练按验证文档在对应环境开展。

M2 名单导入与座位分配已提供 `#/roster` 管理入口。启动、TSV 样例、验证和后续条件见 [名单分配记录](docs/tasks/m2-participant-seating.md)。

## 等待界面批量控制

管理入口为 `#/waiting`。选择比赛和座位，预览选手、设备与会话后批量进入等待，查看实际完成，再显式结束恢复桌面。服务重启和离线教学期间保持普通环境。独立 8788 开发实例、前台宿主、SCM 安装/卸载和实际验证见 [等待控制记录](docs/tasks/m2-waiting-control.md)。


## 比赛专用账户与即时登录

管理入口为 `#/login`。选择比赛和电脑、预览账户与会话、明确确认后准备并登录，核对实际 SID/桌面后进入等待。显式恢复教学和断线撤回本轮限制并保留当前桌面与文件。文件模拟和 Windows 原生测试能力在界面分别标记。测试机安装、恢复命令、验证结果与环境条件见 [比赛账户任务记录](docs/tasks/m2-contest-login.md)。

## 本机诊断与比赛桌面维护

客户端 `wccs-agent --setup IDENTITY` 可离线导入连接配置、查看网卡、执行实际通道诊断并导出脱敏报告，也可主动显示设备识别画面。管理端 `#/devices` 核对诊断与健康，`#/seats` 筛选健康/版本差异，`#/presentation` 配置等待展示，`#/maintenance` 预览并下发固定桌面、目录检查和脚本部署模板。格式、操作及验收限制见 [machine-tools 对齐记录](docs/tasks/machine-tools-alignment.md)。
