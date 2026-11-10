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

M1 已交付 `wccs-inventory` 和 `wccs-session`。采集器通过原生 Windows API 只读采集系统、账户权限、磁盘卷和网络，并按明确配置路径检查工具文件，输出 JSON 与脱敏摘要。会话原型使用 Win32 窗口展示合成身份、输入提示和退出入口，开发机交互与异常重启已验证。工具候选清单及客户端配置检查已实现，服务端路径管理页面和下发纳入 M2。机房镜像、多屏及系统会话切换仍待现场验证。

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
| [路线图](docs/roadmap.md) | 当前状态、任务顺序和完成条件 |

## 从 Codex 开始下一项工作

可以向 Codex 提交以下任务。

> 阅读 AGENTS.md、需求基线和路线图，推进 M2 的 Linux 服务、设备注册及心跳、管理 UI 基础闭环。明确 Linux 验证环境，固定应用工程版本，首先交付工具路径管理、鉴权、版本化下发和检查回传的可验收切片。

机房真实复原、设备策略和自动登录演练按验证文档在对应环境开展。
