# machine-tools 能力对齐

开始日期为 2026-11-12。沿用 `feat/waiting-screen-design`，保留开始时的等待界面、等待加固、数据库恢复和预览隔离改动。参考副本 `local/references/xcpc-tools-audit` 的 HEAD 已核对为 `cfb5cc8a7c4df82c29be6354adf2997b5f9258c6`，根许可为 AGPL-3.0。新增实现依据功能和协议需求编写，采用上游源码或素材时逐项补充出处。

## 实施与验收顺序

| 切片 | 核对起点 | 本轮状态 |
| --- | --- | --- |
| 本机配置与诊断 | 已有身份文件、HTTP worker、WSS、SCM、会话 IPC、inventory | 已接通开发机与浏览器，独立 Windows SCM 待实测 |
| 快速配置与现场识别 | 已有服务端 IP 匹配、冲突处理和核对流程 | 文本/文件与识别任务链路已接通，原生完整验收待独立环境 |
| 健康上报与座位图 | 已有静态 inventory、设备和座位详情 | 开发机实际采集、上报和浏览器通过，SCM/Session 0 待验证 |
| 等待展示数据 | 已有原生展示、身份快照及单调控制租约 | 配置/API 与浏览器通过，原生完整画面和真实 DOMjudge 待验证 |
| 批量桌面与脚本任务 | 已有工具检查、等待/登录预览、逐机结果和事件 | 模板、预览、执行适配与回执已接入，独立 Windows 账户/文件验收未完成 |
| 新增任务恢复 | 已有工具 pending、凭据轮换和控制恢复日志 | 日志、ACK、回退与进程树管理已接入，真实系统重启/进程树验收未完成 |

本轮先交付文本与文件配置入口。网卡用于明确座位核对目标和展示当前 IP、MAC、网关、DNS。网络参数修改和二维码扫描为可选适配，当前先推进核心配置流程。

## 验证记录

基线检查通过后记录实际命令和结果。源码审查、模拟故障、开发机运行、独立 Windows 测试和 Linux 原生验证分别记录。每个切片须贯通正常入口与管理页面后才能标记完成。

## 本机配置与诊断

入口为 `wccs-agent --setup IDENTITY`，提供配置与网卡查看、JSON 粘贴、文件导入、差异预览、取消、离线保存和诊断包导出。非交互入口为 `--setup-status IDENTITY`、`--configure IDENTITY CONFIG BASE_REVISION`、`--diagnose IDENTITY SERVICE OUTPUT`。开发机验证使用显式 `--diagnose-test IDENTITY SESSION_EXE OUTPUT`，服务器和页面保留 `local_test` 证据类别。

配置格式为 `{"schema_version":1,"server":"https://wccs.example.test","seat_claim":null}`。服务器地址可为 null，逐机身份材料中的初始 server 可为空字符串。座位核对信息包含 room_id、seat_number、interface_index 和 mac_address，保存时再次核对真实网卡。配置保存在身份文件旁的 connection.json，使用独占配置锁、基准版本和原子替换，绑定设备及注册材料。重复保存相同配置保持版本，版本冲突要求重新预览。

HTTP worker 和 WSS 宿主读取同一有效配置。地址变化使旧 WSS 连接退出并执行既有恢复流程。教学离线保存不要求服务器在线。诊断依次核对本地配置、SCM 状态、会话鉴权 IPC、服务标识、注册/认证、HTTP 心跳和独立 WSS 随机关联号回执。会话探测由独立服务工作线程调用原 Child/命名管道实现，既有控制续租独立执行。

诊断结果先持久化，单份待传内容保留到服务端精确 ACK。后台每轮最多补报一份，同编号不同内容返回冲突，设备身份代次变化隔离旧报告。管理端设备详情展示逐步骤状态、最近成功时间、配置版本、失败恢复说明和离线过期标记。导出包仅选取固定结果字段、配置摘要、网络信息与白名单事件，凭据、任意异常文本和原始日志被排除。

### 已执行验证

- 基线 `check-m2.ps1` 的 Rust 部分通过，数据库部分遇到本地 PostgreSQL 停止导致的 10061。按既有命令启动独立 `local/m2/postgres` 集群后继续验证。
- `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 通过发布构建、Clippy、Rust 常规回归、10 项 PostgreSQL 综合回归和 35 项前端测试及生产构建，日志为 `artifacts/machine-tools-diagnostics-m2.txt`。
- 新增回归覆盖配置版本冲突、相同保存重试、身份变化、敏感字段拒绝、离线导出脱敏、重复诊断、编号冲突、跨设备上报拒绝、诊断前后既有控制连接及命令保持有效、身份代次变化后的报告隔离。
- 隔离 8790 / machine_tools_browser 实例实际执行 `--configure` 和 `--diagnose-test`，配置、注册、HTTP、WSS 和真实原生命名管道均获得成功回执。SCM 明确显示未知，证据保存在 `local/machine-tools/report-online.json`。
- 真实浏览器从登录、设备清单、查看详情进入诊断表，核对同一合成设备的配置 v1、开发机测试类别、逐步骤回执及超出在线期限后的过期提示。

Windows 10 SCM/Session 0、可信 HTTPS/WSS、真实机房网卡及 Linux 原生仍待对应环境实测。回环 HTTP/WS 和开发机 IPC 的结果按上述环境记录。

## 快速配置与物理座位识别

管理端入口为“批量维护”，也可从设备诊断详情进入单机识别。预览显示固定目标、当前服务端座位/IP、客户端能力和跳过原因。下发后保存任务内容摘要、设备身份代次、地址版本、会话及宿主实例，逐机阶段与请求事件关联。取消请求令续租停止，页面等待实际停止回执。

本机入口为配置菜单第 5 项，或 `wccs-agent --identify IDENTITY SESSION_EXE SECONDS`。时长为 5～120 秒，使用现有鉴权 IPC 和多屏原生窗口，支持 Esc、按钮关闭与单调计时到期。标识中的本机座位声明显示“待核对”，服务端 IP 绑定仍通过现有核对与换机流程处理。识别与正式等待共享窗口互斥，同时在预览/提交事务核对互斥任务。

独立维护工作线程保留 accepted、executing 和终态日志，服务端精确确认后标记 ACK，保留完成凭据防止重放。进程重启将未完成日志改为 interrupted，断线或授权结束停止识别。正文、状态和输出采用固定有限枚举。

- `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 通过 Windows 发布构建、Clippy、常规 Rust 测试、11 项 PostgreSQL 综合回归、35 项前端测试及生产构建，日志为 `artifacts/machine-tools-identification-m2.txt`。
- 新增 PostgreSQL 回归通过维护预览/提交、等待任务互斥、相同请求重试、编号内容冲突、相同阶段不同结果拒绝、取消与终态回执、身份代次变化隔离。客户端回归通过重启中断及不重放副作用。
- 开发机曾启动识别窗口并观察到窗口存在。用户说明正在游戏后，已关闭本轮创建的窗口与测试进程。完整图像、按键、超时和服务边界验证转到独立 Windows 测试目标，当前仅记录窗口创建证据。

## 健康上报与等待展示进展

动态健康采集复用 inventory 的 Observation 模型，新增每项采集时间。Windows 适配直接调用 GetSystemTimes、GlobalMemoryStatusEx、GetTickCount64、显示配置枚举及 Raw Input 设备枚举。CPU 首个样本保留未知。镜像版本来自身份文件旁 deployment.json 的 schema_version=1 和 version，未配置保留 not_configured。独立工作线程约每 30 秒采集并上报，慢采集延长间隔，失败保留本地最新样本。服务端按设备身份代次、当前宿主实例和递增序号核对。管理端详情展示完整样本，座位图增加健康未知/异常、过期和版本差异筛选，版本基准由工作人员在筛选器填写。

等待展示使用独立 HTTP 数据通道和工作线程。配置按比赛版本化，包含开赛时间、DOMjudge API 根地址与比赛编号、学校名对应校徽。校徽格式当前为 96×96 以内的 24 位无压缩 BMP，检验尺寸、步长、内容长度和 SHA-256，随内网配置交付。缺失时显示原默认图标并按刷新周期重试。新增配置、健康和展示均复用已锁定依赖，服务端直接引用已在锁文件中的 url 2.5.8 做 URL 校验，reqwest 仍为服务端测试依赖。

展示缓存保存比赛/设备/座位及身份版本、来源和配置摘要。本机配置状态提供脱敏缓存诊断。原生界面从鉴权 IPC 获得展示数据，按命令身份核对，倒计时使用服务器样本与 Windows 单调启动计时，时钟偏差和 RTT 单独显示。F1 展开只读工作人员诊断。控制授权继续使用原协议，展示读取不会取消或续延命令。

DOMjudge 适配执行 GET API_ROOT/contests/ID，核对 HTTP 状态、JSON 比赛 id 和 name，拒绝重定向并限制读取长度。401/403、网络失败、无效响应和未配置分别呈现。接口依据 [DOMjudge API 文档](https://www.domjudge.org/docs/manual/main/develop.html)，实际赛场实例仍待提供或接入验证。

- `cargo check --locked --workspace --all-targets --target x86_64-pc-windows-msvc` 通过。
- `cargo test --locked -p wccs-server --target x86_64-pc-windows-msvc --test http database_tests -- --ignored --test-threads=2` 通过 6 项数据库恢复、500 座预览隔离、诊断隔离、维护、健康和展示综合回归，日志 `artifacts/machine-tools-presentation-postgres.txt`。健康参数首次验证发现 bigint 绑定推导问题，已显式标注类型并复测通过。
- 健康和展示前端检查通过，展示新增后的前端为 36 项测试，日志 `artifacts/machine-tools-presentation-web.txt`。
- 用户游戏期间使用后台编辑与限并发构建。原生交互验证安排在独立 Windows 目标。

## 桌面模板与竞赛脚本包

入口 `#/maintenance`。展开“桌面与脚本维护配置”，粘贴或导入 schema_version=1 配置，包含 tool_version、work_directory、desktop_directory、script_directory、links 和可选 package。工具 ID 从当前有效服务端配置解析成完整路径，发布与任务预览都核对配置版本。全场管理员发布配置，授权操作员只能对其范围内设备下发模板。

正常流程为：准备并核验普通比赛账户桌面，发布工具与维护配置，选择设备和模板，核对预览的执行账户/完整路径/跳过原因，提交，再核对逐机阶段、结果和关联事件。未声明能力、无普通账户真实桌面、离线、正在等待及已有维护任务分别跳过。开发机 test_mode 只声明识别能力，不能用它伪装 SYSTEM 桌面或脚本部署成功。

| 固定模板 | 实际行为与成功依据 |
| --- | --- |
| prepare_desktop | 根据真实普通账户令牌解析桌面，只操作所属 WCCS 子目录；创建或更新工具/资料 .lnk、网站 .url；文件内容读回及普通账户读取成功 |
| check_workspace | 核对 native 配置工作目录、账户所有权/SID/非管理员令牌，按完整工具路径检查读/执行与目录读写 |
| deploy_scripts | 核对解释器完整路径与 PE 文件版本，拒绝普通账户可写的脚本根目录，校验不可变包版本，最后原子更新 active.json |

打包入口 `python scripts/package-contest-scripts.py --help`。清单版本 1 包含包 ID、版本、配置版本、运行工具 ID/文件版本、来源、许可及逐文件 SHA-256。当前限制为 UTF-8、64 文件、128 KiB 原始内容；HTTP 请求总大小仍为 256 KiB。受限名称、重复大小写路径、重解析点、保留 manifest.json 和超限输入被拒绝。包作为数据部署，不执行远程任意 Shell。compilecpp/submit 的独立命令实现、终端入口及实际 OJ 提交仍按 ADR 0003/M3 另行验收。

安装脚本为新安装写入 configure.cmd，工作人员以安装管理权限启动后进入本机菜单。导入的 connection.json/deployment.json 可随明确逐机身份一起安装。已有身份材料与共享镜像必须分开管理，不复制一台设备的凭据给其他机器。未知服务器地址可为空，联网前保留待验证；设备注册继续使用服务端独立授权材料。

## 新增任务执行与故障恢复

客户端在任何模板副作用前保存 accepted；执行阶段和终态先写盘再上报。ACK 核对 job_id 与 stage，终态记录保留，重复派发不会再次执行。服务端拒绝同请求不同内容、同阶段不同结果、终态覆盖和跨身份代次结果。

文件事务先保存全部旧值和新内容，逐项原子替换、读回并核验普通账户访问，最后提交。脚本激活指针是最后一个修改项。未提交事务重启后回退；已提交但回执未报的事务先核验实际内容再补报。所有权标记和已创建的空目录可能保留，用于后续重试确认归属；不会清除无关目录。回退发现外部修改时保留文件，终态报告 rollback_failed，本机阻止后续维护并继续尝试恢复，直到工作人员解决占用/内容冲突。历史终态保持不可变。

识别失联、取消或身份变化即停止，5～120 秒使用单调期限。文件任务在各步骤和最终提交前检查授权及整体 60 秒期限，取消/失联/配置变化回退未提交写入；同步 Windows 文件系统调用本身不能由该期限强制打断，驱动挂死仍是独立故障验收项。会话子进程采用挂起创建、加入 Job Object 后恢复，停止或服务崩溃关闭 Job 时结束整个受管进程树。进程树实际 Windows 10 证据尚缺。

脱敏诊断包增加最近 32 项任务的编号、阶段、固定结果代码、ACK 和阶段结果；不导出脚本内容、选手字段、文件路径或任意步骤错误文本。

## 本轮组合验证与环境边界

- Windows 开发机 `--health local/machine-tools/device.json` 实际执行采集，单次记录 1 ms。随后后台 test_mode 宿主的真实健康 HTTP 上报被管理端接收；浏览器核对 CPU/内存/显示信息、键鼠计数、部署版本未配置和逐项时间。证据 `artifacts/machine-tools-health-native.json`，不等同于 SCM/Session 0 采集证据。
- 使用隔离 8791 代理仅将健康请求延迟 8 秒并返回 503，客户端 5 秒超时；`python local/machine-tools/verify-slow-health.py` 取得 40 次真实 WSS 控制续租，最大控制样本年龄 1.918939 秒，40 个不同续租时间。证据 `artifacts/machine-tools-slow-health.json`。测试结束已停止代理与 test_mode 宿主并恢复配置地址。该测试覆盖慢上报/失败；真实慢设备驱动采集仍待环境验证。
- 真实后台浏览器完成维护配置 v1 发布、工具路径预览、能力不足与离线跳过；展示配置 v1 保存；健康详情、座位图版本基准筛选和离线数据过期。没有向开发机下发识别、账户或文件修改任务。截图 `artifacts/machine-tools-maintenance-browser.png`、`artifacts/machine-tools-presentation-browser.png`、`artifacts/machine-tools-health-browser.png`。
- 新增数据库回归覆盖维护配置工具引用、版本冲突、重复请求、查看者拒绝、普通账户能力门槛、固定命令快照、重启回退结果补报与终态不可覆盖。展示回归覆盖独立查询不会改变既有控制、身份/配置版本核对。文件回归面向独立临时目录，覆盖部分写入、访问核验失败回退、外部内容保留和已提交内容重启核验。
- `python -m unittest discover -s scripts/tests -v` 通过 16 项。新增打包回归验证保留原始 CRLF 字节及 SHA-256、保留名称和体积上限。
- Windows 独立目标使用既有 WCCS-Login-Test / MachineGuid 8b041e9f-3bfc-432d-a620-7ca446dde702 的新 qcow2 覆盖层，原始磁盘未改。无窗口、单核、低优先级启动。WHPX 在系统更新重启阶段报 `Unexpected VP exit code 4` 并暂停；TCG 回退继续经历 Windows 更新，随后黑屏，120 秒受保护探针超时，未取得 guest 实际回执。日志和覆盖层在 local/machine-tools-vm。不能将之前登录切片的旧 VM 结果计为本轮新行为验收。
- WSL 查询确认未安装。Linux 交叉检查 `cargo check --locked -p wccs-server -p wccs-protocol --target x86_64-unknown-linux-gnu` 被 ring 的 C 编译器缺失阻断（x86_64-linux-gnu-gcc）；该结果记录为未通过，不沿用早期切片的交叉检查结论。Linux 原生、systemd、HTTPS/WSS 和负载验证需要真实 Linux 目标。

### 剩余原生验收步骤

1. 在可正常启动的独立 Windows 10 镜像安装成套 agent/session 和 configure.cmd，运行 `--diagnose IDENTITY SERVICE OUTPUT`，逐项核对 SCM PID、用户会话 IPC、可信 HTTPS/WSS；从管理页核对同一报告。
2. 通过现有比赛账户页面完成普通账户桌面确认，发布真实工具完整路径及维护目录，分别从批量维护页面执行三种准备模板。核对 .lnk/.url、普通账户读取执行、脚本版本目录/摘要/active.json，检查权限拒绝与不可用目录的逐机失败。
3. 对独立目标注入断线、取消、文件占用和第三方文件修改；在预写日志后、部分写入后、激活后但 ACK 前中断服务，再重启服务和系统。核对文件旧值、结果补报、重复派发无副作用，以及未解决回退阻止新任务。
4. 在 VM 内验证识别多屏、Esc/按钮/到期、等待互斥、会话变化；使用专用可派生孙进程的测试程序核验 Job Object 终止整棵进程树。验证 F1 诊断、校徽正常/缺失/损坏/重试、倒计时、时钟回拨、断线与重新联网不恢复旧授权。
5. 接入真实 DOMjudge 测试实例公开 API，分别核对可用、认证要求、实例不可用和错误比赛编号。没有实例时保留未知/失败，不用页面 URL 可访问代替 API 成功。
6. 在 Linux 执行 `python3 scripts/check-linux.py`，配置独立 PostgreSQL 后跑集成测试，随后验证 200～300 台续租、500 目标预览、TLS 与部署恢复。

### 最终检查（2026-11-12）

| 实际命令 | 结果与证据 |
| --- | --- |
| `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` | 通过固定 MSVC/SDK 发布构建、格式、Clippy、80 项 Rust 常规、14 项 PostgreSQL 综合、Credential Provider 构建、npm ci、36 项前端及生产构建；artifacts/machine-tools-final-m2.txt |
| `npm --prefix web run check`（实际在 web 目录运行 npm run check） | 类型、lint、格式、36 项测试与构建通过；artifacts/machine-tools-final-web.txt |
| `python scripts/check_repo.py` | 工作区 197 文件通过 |
| `python -m unittest discover -s scripts/tests -v` | 16 项通过；artifacts/machine-tools-final-python.txt |
| `git diff --check`、`git diff --cached --check` | 均通过 |
| PowerShell Parser.ParseFile 检查 scripts/waiting-service.ps1 | 语法通过；安装与 ACL 实际行为仍待 VM 验收 |
| `wccs-agent --diagnose-test local/machine-tools/device.json target/x86_64-pc-windows-msvc/release/wccs-session.exe local/machine-tools/report-final.json` | 最新发布二进制实际 IPC、注册认证、HTTP 和 WS 回执通过、服务端 ACK=true；SCM/安装明确未知。探针在创建窗口前返回，不显示原生窗口；artifacts/machine-tools-final-diagnostics.txt |
| `wccs-agent --setup-status local/machine-tools/device.json` | 返回配置 v3、最新诊断和本机摘要，无凭据；local/machine-tools/status-final.json |
| `cargo check --locked -p wccs-server -p wccs-protocol --target x86_64-unknown-linux-gnu` | 未通过：缺少 x86_64-linux-gnu-gcc；artifacts/machine-tools-linux-cross.txt。未声称 Linux 原生验证完成 |

本轮停止了自建无窗口 VM、VM 探针服务、慢上报代理及后台测试宿主，移除本轮未执行的只读 VM 探针请求。保留独立覆盖层、日志和隔离数据库供复现；原 VM 基础磁盘未改。最终测试服务也已停止。未执行 Git 提交或推送。

六项均已形成代码与正常页面/本机入口；按验收标准仍不能标记全部完成：第 1～3 项缺独立 SCM/会话与机房设备验证，第 4 项缺完整原生画面/资源及真实 DOMjudge，第 5～6 项缺普通账户桌面/脚本真实副作用、系统重启和进程树矩阵。以上步骤列为下一项可执行工作，不能用旧登录切片证据替代。
