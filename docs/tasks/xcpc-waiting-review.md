# 项目审查与 xcpc-tools 等待界面替换

日期为 2026-11-12。用户要求审查现有项目，并将 waiting_screen 换成 xcpc-tools 中的展示界面。沿用 `feat/waiting-screen-design`，工作区已有等待加固、协议、管理页和文档改动，本次在其基础上修改会话窗口并同步文档。

## 审查发现

### P1 数据库连接中断后服务无法自行恢复（已修复）

原实现中，`crates/wccs-server/src/lib.rs` 的 `connect` 仅创建一次 PostgreSQL Client，连接驱动退出后记录 `database_connection_closed`，AppState 继续保存同一个已关闭 Client。数据库恢复后请求仍使用该连接，需重启服务恢复。

初次审查在独立 `waiting_browser` 测试实例复现。先确认管理 API 返回 200，再通过 PostgreSQL 的 `pg_terminate_backend` 精确断开本次启动服务的连接，独立 `SELECT 1` 成功，约 1、6、16 秒后的 `/api/state` 均返回 500，重启本次测试服务后恢复 200。原始证据为 `artifacts/xcpc-database-recovery.txt`。

2026-11-12 后续修复已加入连接失效重建、握手超时及失败冷却，验证原进程恢复、旧事务回滚、请求编号回放和设备重新授权。浏览器验证停止并重启 PostgreSQL 后原登录页面恢复，见 [修复记录](database-recovery-preview.md)。

### P2 等待预览与设备心跳竞争全局数据库锁（已修复）

原实现的 `AppState.db` 使用 `Mutex<Client>`。`waiting.rs` 的 `plan` 在该锁和事务内逐座位获取选手、设备、最近命令与控制状态，最多接受 500 个座位。`control.rs` 的 `exchange` 同样取得此锁。慢查询或大批量预览会阻塞设备续租处理，可能触发客户端的安全离线退出。

2026-11-12 后续修复将计划改为批量读取，等待预览使用独立连接及只读可重复读快照，提交保留写栅栏和最新权限核验。500 座位预览被数据库表锁阻塞时，真实 WebSocket 心跳仍完成往返。目标 Linux 服务器的 200～300 台并发续租及批量提交容量继续按 [修复记录](database-recovery-preview.md) 补验。

### P2 等待界面的长文本与竖屏可读性

原生窗口先前沿用固定横屏画布，`fitted_font` 可将极端长姓名、学校缩小到 1 像素。本轮改为按实际宽高布局，保持最小字号并使用省略号，原生控件保留完整文本。回归覆盖中文、连续英文、小屏、竖屏及 4K，确认姓名和座位区域分离。

### 后续产品完善

当前 `WaitingIdentity` 提供赛事、姓名、学校与座位文字。校徽资源、服务器校时和比赛起始时间需要贯通管理配置、身份快照及离线资源交付。M3 可参考 xcpc-tools 的名单字段映射、账号导出预览和 IP 缺失 / 歧义统计，接入真实 DOMjudge 与换机撤销验证。

## 本轮界面交付

- 参考固定提交 `cfb5cc8a7c4df82c29be6354adf2997b5f9258c6` 的 `PresentationPage.tsx`、`ParticleBackground.tsx` 和 `style.css`。采用 Rust / Win32 原生绘制，布局为顶部比赛与时钟、中央学校缺省图标、下方左座位右姓名学校的身份卡片。[上游参照](https://github.com/hydro-dev/xcpc-tools/tree/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/src)
- 使用上游触碰提示图及中英文提示，素材转换为内嵌 BMP，来源与许可见 [素材说明](../../crates/wccs-session/assets/README.md)。所有运行资源随会话程序离线交付。
- 300 个装饰粒子，最高 20 FPS，Windows 关闭客户区动画时使用静态背景。覆盖提示显示期间暂停背景重绘，双缓冲及兄弟窗口裁剪处理闪烁和重绘交叠。
- 连续触碰将提示延长到最后一次输入后三秒。布局变化生成的同位置鼠标消息通过瞬时坐标去重，聚合记录仅保存类别计数。连续输入时文字计数按原 250 毫秒刷新周期更新。
- 当前学校使用缺省图标，时钟显示本机时间。控制租约和提示消退继续依据单调计时。正式模式的键盘防护、前台核验、退出限制、鉴权 IPC 及宿主恢复沿用当前实现。
- 仅扩展现有 windows 依赖的 Controls / SystemInformation feature，用于 owner-draw 结构和时间查询。

## 实际验证

| 检查 | 结果 |
| --- | --- |
| `pwsh -NoProfile -File scripts/check-rust.ps1` | 固定 MSVC / SDK 发布构建、格式、Clippy、66 项常规 Rust 测试及 Credential Provider 构建通过 |
| `cargo test --locked -p wccs-server --target x86_64-pc-windows-msvc --test http -- --ignored` | 使用 `local/m2/server.json` 的测试数据库连接环境变量，7 项隔离 schema 的 PostgreSQL / HTTP / WebSocket 综合测试通过 |
| `npm --prefix web run check` | 类型、lint、格式、34 项测试和生产构建通过 |
| 原生布局与素材 | 640×480、800×600、1080×1920、1920×1080、3840×2160，中文与英文长文本、最小字号、省略样式、布局边界及 BMP 格式回归通过 |
| 真实控制入口 | 在独立 8788 / waiting_browser 实例，以 `wccs-agent --foreground ... --test-mode` 经服务端预览、提交、鉴权 IPC 进入新界面，回执 `succeeded` / `waiting` / `ui_observed` |
| 键鼠与结束 | 实屏核对合成姓名、学校和座位，键盘触碰触发原版图片覆盖层，空闲三秒消退；提示显示期间管理端结束成功，回执为桌面，窗口退出 |
| 仓库检查 | `python scripts/check_repo.py` 检查 155 个文件，14 项 Python 回归和工作区 / 暂存区 diff 检查通过 |

构建日志为 `artifacts/xcpc-waiting-rust-check.txt`，数据库集成为 `artifacts/xcpc-waiting-integration.txt`，前端为 `artifacts/xcpc-waiting-web-check.txt`。窗口与提示截图分别为 `artifacts/xcpc-waiting-controlled.png`、`artifacts/xcpc-waiting-warning.png`。

当前实屏证据来自开发机受控 test 模式。Windows 10 正式受控模式、多屏混合 DPI、长时间动画资源消耗及机房复原镜像需继续验证。数据库恢复和等待预览锁竞争已完成后续修复及故障回归。下一项验证目标 Linux 环境的多设备并发容量。

本轮启动的前台测试宿主、会话窗口及 8788 测试服务已退出。截图保留在忽略的 artifacts 目录，合成设备和比赛记录保留在既有独立测试 schema。

## machine-tools 完整复核

以下对照表记录本轮实施前的缺口。2026-11-12 后续已按六个切片接入代码、页面和回归；本机诊断、健康与管理端已获开发机实测，独立 Windows、DOMjudge 和 Linux 验收仍有缺口。最新入口、80 项 Rust/14 项数据库/36 项前端/16 项 Python 验证及剩余步骤见 [machine-tools 对齐记录](machine-tools-alignment.md)。

2026-11-12 用户要求扩大参考范围，补充检查 machine-tools 全部前端工具模块、配套 Python 探针、systemd 单元、服务端展示数据和命令管理入口。远端 main 核对为 `cfb5cc8a7c4df82c29be6354adf2997b5f9258c6`。本节为源码对照结果，后续切片顺序为建议。

### 可以落实到 WCCS 的内容

| 借鉴项 | 上游实际实现 | WCCS 当前基础与具体增量 |
| --- | --- | --- |
| 现场配置入口 | SetupPanel 汇总座位、服务器地址、上报配置和服务状态，提供保存、测试、完成流程 | 当前通过身份文件、CLI 和 PowerShell 安装服务。增加本机配置向导，支持服务器地址晚于镜像确定的现场条件，呈现保存、注册、服务运行和服务器接收的各阶段结果 |
| 端到端连通性诊断 | HTTP 上报与 WS 上报分别测试，WS 收到 welcome 才成功，十秒超时；检查 systemd 实际 active 状态 | 将现有设备注册、HTTP 心跳、WSS 认证、SCM 与会话 IPC 组合为可单独执行的诊断步骤，显示失败环节和最近成功时间，汇总到管理页 |
| 快速配置及变更预览 | xms1 载荷包含座位、服务器、IP/前缀、网关和 DNS，有限长校验，展示待修改字段，由操作者选网卡 | 借鉴配置包导入及预览交互，Windows 适配通过受认证服务完成。座位提议与服务端 IP 绑定核验衔接。二维码作为可选输入方式，现有工具路径继续由服务端配置 |
| 大字座位现场核对 | 放大座位显示，完成配置后显示座位与 IP | 增加工作人员主动打开的设备识别画面，从管理端选机器或本机诊断入口触发，核对物理座位、IP 和设备 ID。该增量直接服务 REQ-03 的现场识别与换机 |
| 机器健康与镜像版本 | MachineInfo 和探针采集 CPU/内存使用、uptime、镜像版本、显示器、网卡及键鼠状态；管理端提供硬件状态座位图 | inventory 已有 Windows、账户、卷、网卡、DNS/网关和工具文件信息。补充动态健康与镜像版本上报，将异常映射到座位图，支持先处理缺少键鼠、版本不一致和资源异常设备 |
| 展示时间与连通状态 | PresentationPage 用 serverTime 与请求往返时间估计时钟偏差，展示开赛倒计时；Tools 与 OJ 分别显示连接状态 | 当前等待屏显示本机时钟和缺省校徽。贯通比赛开始时间、校时样本、WCCS / DOMjudge 状态及诊断详情。控制租约继续使用单调计时和明确授权 |
| 学校图片及展示缓存 | 服务端按学校匹配 WebP，限制体积、检查格式并以 SHA-256 命名；客户端校徽回退/重试，展示缓存绑定服务器 URL 和座位 | 复用资源清单与内容摘要的思路，将校徽随比赛资料交付，标记资源缺失；本地缓存绑定比赛、身份版本和来源。缓存可以支持资料预览与诊断，断线控制恢复沿用现有状态机 |
| 批量维护任务与逐机输出 | Commands 提供模板、选择目标、在线/离线和协议能力说明、任务历史；Python 探针回传退出码、stdout/stderr | 工具检查与等待控制已有目标预览、跳过原因和阶段回执。M3 桌面准备、脚本部署可扩展同一任务模型，增加版本固定的操作模板、输出摘要与逐项执行证据 |
| 执行恢复的工程细节 | 探针执行前写 running，结果写 outbox，收到 result-ack 后清除；重启将中断任务转为失败结果；600 秒执行上限、每路 64 KiB 输出上限、清理进程组 | WCCS 已有工具结果持久回传、控制记录和带抖动重连。下一批多步骤任务借鉴持久结果队列、输出限额和中断结果模型，Windows 子进程组由 Job Object 等适配实现，明确可重试步骤及已有副作用 |

现场配置、诊断与快速配置实现位于 [SetupPanel](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/src/components/SetupPanel.tsx)、[probe.ts](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/src/utils/probe.ts)、[QuickConfigModal](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/src/components/QuickConfigModal.tsx) 和 [quickNetwork.ts](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/src/utils/quickNetwork.ts)。

机器状态与任务执行实现位于 [MachineInfo](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/src/components/MachineInfo.tsx)、[探针](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/scripts/machine_tools_probe.py)、[ArenaView](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/ui/app/components/ArenaView.tsx) 和 [Commands](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/ui/app/pages/Commands.tsx)。

展示数据与素材链路位于 [PresentationPage](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/src/components/PresentationPage.tsx) 和 [presentationRoster](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/server/service/presentationRoster.ts)。

### 适配时需要核实的实现边界

上游工具依赖 Linux 镜像预装的 Python 探针、systemd 服务、Netplan、Neutralino 及 machine-setup-helper。当前仓库的 privileged.ts 查找外部预装 helper，其完整实现需从镜像工程核对。Windows 对应能力应接到现有 SCM、SYSTEM 服务、受认证 IPC 和安装包中。[镜像依赖说明](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/README.md)

网络预览会在应用前再次比较原配置及网卡身份，值得移植。当前 applyNetwork 在写入覆盖文件后调用 netplan apply，失败时报告已经写入的部分结果。Windows 网络设置候选功能还需要设计旧值记录、连通性验证和失败恢复。[网络应用实现](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/packages/machine-tools/frontend/src/utils/quickNetwork.ts)

上游 Python 探针执行服务端下发的 shell 文本，普通连接断开后执行任务可继续运行，离线目标也可在重连后领取待执行命令。WCCS 的比赛控制任务应由既有权限、期限、连接与实例代次约束，多步骤维护任务再逐项规定失联后的处理方式。[探针执行与重连](https://github.com/hydro-dev/xcpc-tools/blob/cfb5cc8a7c4df82c29be6354adf2997b5f9258c6/scripts/machine_tools_probe.py)

上游 outbox 在收到 ACK 后移除结果，其本地命令去重覆盖正在执行及待确认结果。未来 WCCS 通用任务需要结合服务端持久请求编号、设备执行记录和副作用恢复设计。上游上报认证使用配置 reportToken，当前 WCCS 已有独立设备凭据与代次，可以作为适配基础。

### 建议实施顺序

1. 本机配置与诊断向导，贯通地址导入、服务状态、注册与 HTTP/WSS 测试、大字座位核对和可带回的脱敏诊断报告。首个可验收结果为志愿者在一台合成设备完成接入并从管理页面确认同一设备。
2. 健康上报和座位图异常视图，先实现镜像/客户端版本、键鼠、资源状态及采集时间，再贯通现场核对结果。
3. 等待展示数据补齐服务器时间、倒计时、校徽和 WCCS / OJ 状态，接入比赛配置与资源交付。
4. 随 M3 桌面与脚本切片完善版本化任务模板、持久结果队列及输出限额。

以上结论来自固定提交的源码审查和 WCCS 当前实现对照。上游 Linux 镜像运行行为、真实机房网络修改和新增 Windows 适配均需各自的实现及运行验证。本轮仓库文档检查和 diff 检查通过。
