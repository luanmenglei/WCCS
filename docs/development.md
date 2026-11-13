# 开发规范与 Codex 工作流

## 工作入口

根 AGENTS.md 提供持续项目指令，目录级 AGENTS.md 补充局部约束。Codex 启动时读取项目指令链；更新指令后，当前任务应重新读取相关文件，后续会话从仓库入口开始。[OpenAI 项目指令文档](https://learn.chatgpt.com/docs/agent-configuration/agents-md)

路线图承担项目交接记录，需求与 ADR 保存长期决策。按可验收任务逐次实现、保留验证证据的方式参考官方工作流实践，并按本项目授权范围自主推进。[OpenAI Codex 工作流实践](https://developers.openai.com/cookbook/examples/codex/iterating-development-workflows-with-codex)

## 一次任务的执行流程

1. 检查 Git 状态，读取 AGENTS.md、需求、路线图及涉及的架构章节。
2. 明确本次输入、实现范围和验收条件。复杂任务在 `docs/tasks/` 新建简短记录，包含目标、依赖、实施步骤、检查命令、结果及后续事项。
3. 实现一个可运行的完整切片，贯穿入口、逻辑、存储或系统适配及结果反馈。
4. 运行基础检查与相应功能验证。环境依赖缺失时，完成其余可验证工作，并明确剩余验证条件。
5. 自审失败路径、重试、并发、身份和权限边界，修复发现的问题。
6. 同步需求变更、ADR、路线图及必要任务记录。只有满足验收条件的工作才标记完成。
7. 按当前授权提交或交付改动，报告检查结果和剩余问题。用户授权连续推进时，在该范围内继续下一切片。

单人任务默认由主 Codex 完成。用户要求并行时，按独立目录或适配器拆分，先明确接口、文件归属和集成责任，再安排子任务。接口及迁移由指定集成人统一检查。

任务记录是重要交接事实的载体。小修复通过提交说明和路线图即可表达，避免为例行操作生成重复文档。

涉及管理配置的切片先明确配置由谁维护、客户端接收什么输入、结果如何关联配置版本。自审与测试同时核对这项责任分配。工具路径依据服务端的有效配置快照，M1 本地验证使用同一字段约定的显式文件，见 ADR 0004。

## Git 约定

- `main` 保存可验证基线，功能使用短生命周期分支。已有任务分支继续使用。
- 提交格式为 `type(scope): summary`，例如 `feat(agent): report machine inventory`。
- 单次提交聚焦一个可解释变更，重构与无关格式化分开处理。
- 暂存指定文件，检查暂存区 diff，保留用户已有改动。
- 仓库已设置 `.gitattributes`、`.editorconfig` 和本地 pre-commit hook。新克隆执行 `scripts/setup.ps1`。
- pre-commit 检查暂存快照的文本、文件大小、敏感文件名和文档链接。它覆盖已跟踪内容及本次新增内容，工作区中未暂存的修复不能掩盖暂存问题。
- 本地 hook 可被 Git 参数跳过，远端托管确定后接入同样的检查和分支保护。当前使用本地检查与自审。
- 仓库基础检查限制单文件大小为 10 MiB。大型安装包、运行数据和真实名单放在忽略目录或交付存储。应用引入时提交对应依赖锁文件。

## 代码规则

通用要求如下。

- UTF-8、LF、文件末尾换行、去除行末空白。批处理文件允许 CRLF。
- 使用表达领域含义的命名，优先小而完整的函数和现有抽象。引入接口应有平台边界、测试隔离或多实现的具体用途。
- 将系统时钟、网络、文件和 Windows 操作放在可替换的边界，状态逻辑保持可独立验证。
- 超时、取消、重试次数、并发量及队列容量显式定义。外部输入在可信边界校验。
- 异步操作保留异常及取消语义。日志有事件名和关联编号，敏感内容脱敏。
- 配置给出默认值、范围和迁移行为。协议及持久状态变更考虑旧版本客户端和旧数据。
- 代码注释解释约束和选择，文档记录操作方法及验证证据。

当前技术方向采用以下规则，在创建工程时落实工具配置。

| 技术 | 规则 |
| --- | --- |
| Rust | rustfmt 默认风格、Clippy 检查、Result 错误传播、明确取消和超时、Cargo.lock 入库 |
| Windows FFI | 平台模块集中封装 unsafe，说明安全前提，RAII 管理句柄，验证回调生命周期及 panic 边界 |
| React / TypeScript | 两空格、strict 模式、组件职责清晰、边界数据运行时校验、锁定依赖、接入 lint 和格式检查 |
| PowerShell | 四空格、使用 Verb-Noun 命名、错误终止、参数化路径、文件操作使用 LiteralPath |
| Python 工具 | Python 3.11+、四空格、标准库优先、使用 pathlib 和有检查的 subprocess |

选手竞赛脚本的解释器版本根据机房运行环境单独固定。命令入口应兼容所选 Windows 终端，保持参数边界、标准输入输出与退出码。自行实现按项目规范编写，复用脚本保留来源、版本、许可和可审查补丁，见 ADR 0003。

应用工程落地时同时加入实际格式化、构建和测试命令。根目录的文本检查负责基础约束，语言级语义检查由各工具链提供。

Rust 纯业务模块采用 `#![forbid(unsafe_code)]`。平台适配模块逐处审查必要 unsafe，并启用 `unsafe_op_in_unsafe_fn` 检查。网络输入和正常系统失败通过 Result 传播，热路径中的阻塞文件操作及重计算需要受控调度。

### 工具链与跨平台验证

首个 Rust 工程固定 `rust-toolchain.toml`、Cargo.lock、目标平台与最低系统条件。Windows 客户端使用 MSVC 工具链及 Windows SDK。Linux 服务在对应 Linux 环境构建和运行，共享模块在两个平台验证。

已接入 `scripts/check-rust.ps1`，固定 Windows 开发工具集与 SDK，选择 `wccs-inventory`、`wccs-session`、`wccs-protocol`、`wccs-server` 和 `wccs-agent` 包执行 `cargo fmt --all -- --check`、发布构建、Clippy 和测试。全部构建与测试使用 `--locked` 和 `--target x86_64-pc-windows-msvc`。具体命令见 [采集工具说明](inventory.md) 与 [等待界面任务记录](tasks/m1-session-waiting.md)。后续 Linux 构建按目标包选择依赖。

前端使用 React + TypeScript + Vite，创建工程时固定 Node.js、提交 package-lock.json，提供 `npm ci`、类型检查、lint、组件测试及生产构建入口。浏览器端到端测试覆盖真实服务端与客户端模拟器交互。

竞赛脚本使用独立的打包与检查入口，固定脚本和配置版本、文件摘要及解释器依赖。M3 加入命令契约测试、普通用户运行验证和发布回退检查，脚本源码修改可独立生成新脚本包。

### Web 管理端交付

按照架构中的页面表实施。每项管理功能同时包含页面入口、服务端鉴权、操作结果和失败反馈。开发时可使用合成数据验证布局，阶段验收需连接实际 API。自动化浏览器检查覆盖权限拒绝、断线与重连、部分失败和重复点击。

前端构建产物通过 Linux 服务交付，内网浏览器加载所需脚本、字体和图片随包提供。业务页面使用服务端状态和版本号处理并发编辑。完成一项管理功能时记录浏览器实测结果和相应任务证据。

## 开发与运维边界

实现 Windows 特权功能时使用测试虚拟机或明确指定的测试机器。仓库检查与环境采集应只读或仅修改仓库配置。重启、账户变更、USB 策略和清盘的实际执行应对应当前任务的具体目标及授权。

用户已允许比赛机器赛后清空 D 盘，产品流程可按此设计。开发期间通过临时目录或测试卷验证删除行为，并检查卷身份、重解析点与部分失败。

凭据和本地配置使用 `local/`、`secrets/` 或环境变量。`.env.example` 仅包含占位值。仓库文件名检查只发现常见凭据文件类型，提交前仍需检查内容。

## 完成条件

- 需求可从正常入口观察，失败路径有明确反馈。
- 当前切片所需检查通过，无法完成的现场验证有具体条件与记录。
- 没有遗留的临时调试、真实凭据或无关文件。
- 关键改动具备适合其风险的回归验证，相关文档同步。
- 交付说明能让下一次 Codex 会话继续工作，无需依赖聊天历史。

### M2 开发检查入口

`pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 执行固定 Windows 检查、隔离 schema 的 PostgreSQL HTTP 集成、npm ci 和前端检查。配置文件必须指向独立测试数据库。数据库连接通过临时环境变量传给测试，脚本结束后恢复环境。

Linux 使用 `python3 scripts/check-linux.py` 原生构建与检查。设置 `WCCS_TEST_DATABASE_URL` 后额外执行数据库 HTTP 测试。Windows 上 `cargo check --locked -p wccs-server -p wccs-protocol --target x86_64-unknown-linux-gnu` 提供交叉类型检查证据。详细运行边界见 [M2 记录](tasks/m2-tool-configuration.md)。

### M2 Windows 宿主验证

等待控制继续使用 check-m2 / check-rust 的五包检查入口。前台验证显式使用独立 identity、session exe 与 `--test-mode`，SCM 使用 `scripts/waiting-service.ps1` 的明确服务名和受保护安装目录。真实服务需提升权限、LocalSystem 与独立交互式账户，具体目标、退出方法和证据分层见 [等待控制记录](tasks/m2-waiting-control.md)。

## Windows 登录组件开发

Credential Provider V2 的窄 COM 边界位于 native/credential-provider，采用固定 MSVC 与 Windows SDK，build-login-provider.ps1 已纳入 check-rust.ps1。其余客户端、协议与后端继续使用既有 Rust workspace。账户 FFI 集中在 login_windows，IPC 在 login_broker，回调窗口和 COM 引用在 provider.cpp 管理。新增平台依赖仅启用现有 windows 0.61.3 的 API feature。

原生系统验证仅面向明确授权的独立 Windows 测试机，安装脚本复核机器名及 MachineGuid，操作前保留关机磁盘回滚点。文件模拟适配器显式使用 --simulate-login，模拟证据与原生证据分别记录。

### 固定维护与脚本包开发

`python scripts/package-contest-scripts.py --help` 提供独立 UTF-8 脚本包构建入口，只使用 Python 3.11+ 标准库。显式指定源目录、包/配置版本、解释器工具 ID、解释器 PE 文件版本、来源、许可和新输出路径。输出通过管理端“批量维护”的配置面板导入，发布后重新预览目标。工具路径仍取自服务端工具配置。

示例（均为合成输入）：

```powershell
python scripts/package-contest-scripts.py --source-directory local/scripts-src --id contest --version v1 --configuration-version config1 --runtime-tool-id powershell --runtime-file-version 7.5.0.0 --source project --license MIT --output local/contest-v1.json
```

脚本包仅部署和激活，实际编译/提交命令契约另行验收。新增维护任务不得绕过任务日志、配置版本、普通账户访问检查和文件回退。参见 [对齐记录](tasks/machine-tools-alignment.md)。
