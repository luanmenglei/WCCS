# 首次实机离线客户端交付

## Git 收尾回归（2026-11-16）

用户授权提交并同步远程后，重新验证当前源码。`pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 通过，包含固定工具链构建、格式检查、Clippy、84 项常规 Rust 测试、Credential Provider 构建、14 项 PostgreSQL 集成测试、36 项前端测试、类型检查与生产构建。原生 WSS 外部环境测试按入口约定保持忽略。

`python scripts/check_repo.py` 检查 216 个工作区文件通过，`python -m unittest discover -s scripts/tests -v` 的 21 项回归全部通过，覆盖包摘要、重复执行、错误参数、目标拒绝、目录与文件事务回退、脱敏及专用工作目录准备。`git diff --check` 通过。逐提交执行 `python scripts/check_repo.py --staged` 与 `git diff --cached --check`。提交候选文件与忽略目录内 26 个真实凭据值逐字比对，无匹配。

并发测试单独设置 `WCCS_TEST_DATABASE_URL` 为 `local/m2/server.json` 中的测试数据库连接，执行 `cargo test --locked -p wccs-server --target x86_64-pc-windows-msvc --test http concurrent_control_preview_commit_and_reconnect -- --ignored --nocapture` 通过。默认 300 客户端、15 秒采样，10 次管理循环、4713 次轮询，轮询 P95 604 ms、最大 855 ms，接收 P95 1192 ms、最大 1240 ms。此项在 Windows 开发机运行，测试身份、会话与客户端均为模拟数据。

本地 PostgreSQL 初始停止。首次未指定端口启动因默认监听端口不可绑定失败，随后执行 `& 'D:/PostgreSQL/18/bin/pg_ctl.exe' start -D 'E:/project/WCCS/local/m2/postgres' -l 'E:/project/WCCS/local/m2/git-check-postgres.log' -o '-h 127.0.0.1 -p 55432' -w` 成功，测试完成后执行同一程序 `stop -D 'E:/project/WCCS/local/m2/postgres' -m fast -w` 恢复停止状态。日志保存在忽略目录 `local/m2/git-check-*.log`。本轮 Git 整理未操作 Linux 部署与 503-41 实机，沿用下方最后核验的服务状态与待实机清单。

## 本轮最终诊断与未固化镜像交接（2026-11-16）

用户说明当前安装尚未固化到机房镜像，预计重启复原会撤销本轮 C 盘安装。此前直接安排重启的步骤已纠正，本轮先收齐证据，重启后安装保持的验证顺延至单机测试复原基线明确并保存安装后。当前没有取得实际重启/复原结果，后续统一镜像需先完成封装时的逐机身份和状态处理。

收到北京时间 20:45 的最终 Diagnose 回传，ZIP SHA-256 为 `6faee6055d2acf913eeb1cc206f99e8320c39b6413a0cf7f6ae74c52082555df`。核对目标双身份一致，八项诊断全部 succeeded、diagnostic_exit=0、server_acknowledged=true，二进制摘要匹配，机器 CA/HTTPS 验证通过。客户端服务 Running/Automatic/LocalSystem，login_phase=teaching、teaching_enabled=true，account_recovery_pending=false、policy_recovery_pending=false。

再次通过管理 API 核对设备 registered/online，服务端已收到本次诊断。当前实际会话为教学 Administrator，旧等待命令 cancelled、command_id=null；注册窗口保持关闭，定向断网测试的临时 nft 表已不存在。最终回传、核对报告和服务端状态存入忽略目录 `send-back-20261116-204500-a3da0c28.zip`、`final-diagnostic-review.json`、`final-server-state.json`。

本轮已有实机证据覆盖安装注册、登录组件初始化、普通账户登录、等待/解除、三项维护、断线恢复和诊断导出。已修复单机包装层漏建工作目录并生成 v2。剩余事项包括统一镜像初始化与赛前首次登录准备、保存安装后的原生重启/复原、安装升级故障回退、服务异常下本机 Recover、Provider 撤回及策略冲突矩阵、实际编译提交和长期/多屏环境。用户材料和测试凭据均归档于忽略目录，Git 提交与推送等待明确要求。

## 服务器定向断线与重连通过（2026-11-16）

用户因不便操作机箱背面，明确要求用服务器临时 blacklist 替代拔网线。现场先在比赛账户打开记事本并输入未保存文字，保留 PowerShell 窗口。管理端以 confirm_session_end=false 重新准备同一比赛会话并进入等待。随后 `network-drop-test.py start` 创建独立临时 nftables 表，只 DROP 目标实机来源 IP 到服务器 TCP 443 的流量，现有 SSH 管理链路及其他来源保持可用。规则安装前用 systemd-run 安排 60 秒后删除该临时表，配套回执存入忽略目录。

阻断期间计数器实际记录 52 个丢弃包，开发侧可信 HTTPS 健康检查正常。用户确认等待界面退出，原未保存文字和 PowerShell 窗口均保留。自动解除后，`network-drop-test.py observe` 确认规则不存在、设备和控制连接在线，原等待命令 cancelled，actual=desktop、phase=teaching、teaching_enabled=true，仍为原比赛账户会话且 command_id=null，旧等待未重放。

经 SSH 再核对 nft 表清单与测试前完全一致，自动清理 timer/service 均 inactive/not-found，systemd 日志确认清理服务于北京时间 20:40:08 成功结束，wccs/nginx/postgresql 均 active。原始证据为 `network-drop-record.json`、`network-observation-*.json`、`network-drop-cleanup-verified.json`。这是定向静默丢包的实机证据，保留教学恢复后的原会话。最终诊断已在上述交接完成，重启验证按未固化镜像条件顺延。

## 等待解除与三项维护通过（2026-11-16）

首次进入等待时，实机安全软件显示“系统加固”提示，目标为 wccs-agent.exe 修改比赛账户的 DisableTaskMgr 注册表项。用户说明已点击允许，服务端记录该次连接断开、指令 cancelled，客户端恢复教学权限，用户确认屏幕一直为普通桌面。弹窗阻塞触发控制超时属于结合源码与现象的推断，具体超时日志尚缺。照片及回执在忽略目录 `security-policy-prompt.jpg`、`attempt-01-waiting-enter-*.json`。

允许后以新的明确指令复测，维持现有比赛账户会话，重新准备控制并进入等待成功。第二次 enter 回执 succeeded、state=waiting、monitors=1，照片展示测试选手/测试学校及座位 41，用户确认信息和键鼠触碰提示正确。随后 leave 回执 succeeded、state=desktop，用户确认桌面及操作正常。证据为 `waiting-physical-screen.jpg`、`waiting-visual-review.json`、`attempt-02-waiting-enter-*.json`、`waiting-leave-*.json`。正式镜像需提前核对安全软件对该程序精确策略操作及恢复操作的处理。

通过维护 API 发布 profile 9，使用工具配置 4 的包内 PowerShell 7。`test-maintenance.py` 依次执行 prepare_desktop、check_workspace、deploy_scripts，三个实机任务均 succeeded。普通账户工具读/执行和工作目录读写通过，实际用户桌面 WCCS 子目录快捷方式内容验证通过，用户确认 PowerShell 7 快捷方式可正常打开。

脚本部署采用自生成 `field-smoke/v1` 测试脚本，运行时文件版本固定 7.6.9.500，源内容 SHA-256 为 `55cc05cba2e2fef742ccea88a2eceb72620616c1864077beefcce1e605702965`。脚本、清单及 active.json 均 content_verified，受保护根目录为 `C:\Program Files\WCCS\contest-scripts`。本轮覆盖固定模板和脚本分发机制，实际编译器、compilecpp/submit 与 DOMjudge 继续待验。新旧维护配置和任务回执保存在忽略目录 `preparation-*.json`、`maintenance-*.json`。

## 普通账户登录通过与工作目录修复（2026-11-16）

工作人员明确允许结束教学会话后，从管理端向唯一授权实机提交 prepare_login。首轮已创建受管普通账户，但工具/工作目录访问检查失败，客户端恢复 teaching 状态，教学账户启用。用户在现场执行 `Test-Path 'C:\WCCS-Work'` 返回 False，确认单机包装层漏建工作目录。按照目标机管理员命令创建目录并用 `icacls 'C:\WCCS-Work' /grant '*S-1-5-32-545:(OI)(CI)M'` 补齐本地普通用户读写权限后，用户报告完成。

使用新请求号重新预览并提交，客户端完成普通权限工具读/执行与工作目录读写检查、退出教学会话、Provider 登录和默认桌面确认。最终 stage=succeeded、phase=controlled、session=contest、desktop_confirmed=true、tools_ready=true、recovery_ready=true，实际 SID 与目标账户 SID 一致，教学账户在本轮控制期间禁用。用户现场确认已进入 WccsContest，同时观察到新账户首次登录欢迎界面。首轮失败与重试成功证据分别在忽略目录 `attempt-01-login-command-*.json`、`login-command-*.json`，执行入口为 `switch-contest-login.py execute-confirmed-switch`，回执轮询到桌面确认。

修复 `scripts/field-test.ps1`，PrepareLogin 和 InitializeLogin 均准备明确的独立工作目录，拒绝文件/重解析路径及未经核对的所有者，保留已有文件和权限，为本地 Users 增加继承 Modify。软件回归使用临时目录和内存 ACL 模拟，覆盖首次创建、重复执行、已有文件保留、权限条目不重复以及文件/重解析点拒绝；21 项 Python 检查通过。真实普通账户访问由本次重试成功提供证据，新增包装函数完整执行仍需后续现场复测。

生成修正版 `local/field/20261116-503-41/WCCS-503-41-all-in-one-v2.zip`，单机材料版本 `503-41.20261116.2`，113718557 字节、957 文件，SHA-256 为 `f1030a077168555ccc49fb03222c65b4a7e436e8f9b14f55b65c7ee4ec02d632`。包内客户端保持 field.4，修订外层准备入口和说明。包内运行时 Verify 通过。现场已用精确命令补齐同一目录，可继续使用已传输材料完成本次测试。

用户新增赛前自动完成首次账户初始化的要求，已加入需求基线及验证清单。当前自动创建账户与登录已有单机实测，批量赛前初始化、明确就绪状态及复原后的初始化保持继续待实现/验证。诊断还发现普通 I/O 错误在现有账户失败摘要中定位不足，后续应补充受限错误码导出。

## 首台登录初始化通过（2026-11-16）

已收到 InitializeLogin/Start 实机回传，ZIP SHA-256 为 `c8438e46d4c0f2f68a5f675e3f1b479e46f7791f0b6bd5c99b0c4a0ab288ef1b`。目标双身份匹配，登录初始化与服务启动成功，八项诊断全部 succeeded，`login_configured=true`、`login_phase=teaching`、`teaching_enabled=true`，账户和策略均无待恢复项，程序摘要、机器 CA 及 HTTPS 验证通过。管理端登录预览进一步确认原生适配器的 `recovery_ready=true`、当前为 teaching 会话，比赛账户仍 absent，普通比赛账户登录尚未实测。

为下一步测试通过既有 API 新建独立单座位测试机房和“首次实机测试”比赛，使用合成测试选手，按已核对现场 IP 关联目标。操作脚本与状态证据在忽略目录 `local/field/20261116-503-41/prepare-login-test.py`、`login-test-ready.json`。首次创建比赛调用因误用新比赛版本 0 被乐观锁拒绝，修正为比赛列表当前版本后完成，重复执行已准备阶段通过。

现有全局工具配置版本 3 仍指向已清理虚拟机的编译器目录。本轮通过正常配置 API 发布版本 4，仅向明确实机下发已安装包内 PowerShell 的完整路径，用于登录时的普通账户工具访问验证。服务端已收到该程序 available、文件版本 7.6.9.500 的检查回执，证据为 `login-tools-check.json`。版本 3 保留在配置历史，实际编译器路径和编译提交继续待现场核对。

当前登录预览结果为 `session_confirmation_required`。测试座位、选手和工具检查已准备，待工作人员保存当前教学会话中的文件并明确允许退出该会话后，再提交 prepare_login。现阶段保留教学会话和运行中的客户端服务，注册窗口保持关闭。新回传及核对结果归档为 `send-back-20261116-200849-c0da0d3d.zip`、`login-initialized-review.json` 和 `server-after-login-init.json`。

## 首台安装与联网通过（2026-11-16）

已收到单机安装入口的现场回传，原 ZIP 及只读核对结果归档在忽略目录 `local/field/20261116-503-41/`。回传 SHA-256 为 `b26138be03b13f0c031f679579552e58051c662dab940eccc07e9ee69ff35114`。机器名及 MachineGuid 与预先授权目标一致，Install/Start 成功。配置、安装记录、LocalSystem 服务、会话 IPC、服务器、认证、HTTP 心跳和 WebSocket 八项诊断全部 succeeded，服务 Running/Automatic，客户端成套摘要匹配。机器 CA 存在且有效，HTTPS 叶证书验证通过，服务端确认收到诊断。

通过可信 HTTPS 管理 API 再核对设备已注册、enabled 且在线，观察 IP 与此前现场采集一致。注册历史为 `created`、`automatically_approved`，符合既有首代材料在开放窗口内自动批准规则。先前单机说明中“安装后还需人工批准”的表述应按此纠正，关闭窗口时的首次申请及重新授权仍按既有审核规则处理。本次未重复批准。确认没有其他 pending 设备后，通过 `/api/registration-window` 关闭本次窗口，读回 version=5、open=false、closes_at=0。Windows 客户端服务保留运行，Linux 部署保持原状态。

本机 `login_configured=false`、`login_phase=not_configured`，当前没有待恢复策略。下一步现场运行 `03-prepare-login.cmd`，验证恢复管理员原生登录，再运行 `04-enable-login.cmd` 并回传结果。服务会话中的硬件健康报告显示显示器采集 unavailable/system_error 1168、键鼠数为零，需在实际交互会话和后续设备验证中核对，八项通道诊断通过尚不涵盖硬件枚举准确性。

实际执行了回传 ZIP 的 JSON/双身份/八项结果核对，以及管理 API 的 login、state、设备 history 和 registration-window 操作。证据分别为 `installed-review.json`、`server-after-install.json`、`registration-history.json`、`server-after-window-close.json`。真实账户创建、普通账户登录、Provider、教学恢复和重启复原继续待验。

## 单机一次传输材料（2026-11-16）

用户决定先完成已采集的一台机器，要求全部材料一次传输。已生成 `local/field/20261116-503-41/WCCS-503-41-all-in-one.zip`，113718167 字节，957 个文件。单机材料版本为 `503-41.20261116.1`，成套客户端保持 `0.3.0-field.20261113.4`。ZIP SHA-256 为 `dd442073ff1d3c15f93ab268a71041506c8224e845e5e2579390a292452292cf`，两次打包字节一致。完整客户端清单在 `client/manifest.json`，外层脚本、入口、说明和逐机材料摘要在 `bundle-manifest.json`。

新增 `scripts/field-test.ps1` 复用已有安装、自检、SYSTEM 恢复和诊断脱敏入口。每次系统操作先核对显式目标及管理员身份。`01-install.cmd` 自动采集、预检、安装和启动，`02-diagnose.cmd` 再次诊断，运行结果导出到 `evidence/send-back-*.zip`，仅组合固定操作字段及既有白名单诊断。原始报告留在受保护的 `work/`。`03-prepare-login.cmd` 按需创建本机恢复管理员并用实际 SID 生成配置，同名既有账户须已启用且具备管理员身份；本入口创建账户后的重试核对 SID，凭据使用安全输入。`04-enable-login.cmd` 在工作人员验证原生登录后复用 InitializeLogin。Start、Recover、显式强制恢复、RemoveProvider 和 RollbackInstall 分别提供独立入口。

单机默认使用 `C:\Program Files\WCCS`、`WCCS-Agent`、教学账户 Administrator、恢复账户 WccsRecovery、比赛账户 WccsContest 和 `C:\WCCS-Work`。真实机器名、MachineGuid、注册令牌和服务器准备结果保存在忽略目录。恢复管理员密码在实机运行时输入。首次操作只运行安装入口，回传结果后核对注册申请并批准，再继续账户和比赛控制测试。

实际检查命令及结果如下。

- `python -m unittest discover -s scripts/tests -v` 通过 21 项，新增单机入口的中文/空格路径、重复 Verify、摘要篡改、错误机器及参数拒绝。拒绝测试确认未创建 work/evidence。
- 包内 `client/runtime/pwsh.exe -NoProfile -File field-test.ps1 -Action Verify` 通过，实际 `00-verify.cmd` 通过。开发机运行专用 `01-install.cmd` 按预期拒绝目标不匹配，未执行系统修改。
- 对实际 ZIP 执行 CRC、957 文件清单、内外层摘要核对，排除 SSH 密码、服务器管理员凭据、CA 私钥及本地运行记录。校验结果在忽略目录 `transfer-validation.json`。
- `python local/field/20261116-503-41/prepare-transfer.py` 通过现有管理员 API 预置待注册设备并生成合并包，同材料重复执行与重打包通过。注册材料有效至北京时间 2026-11-17 19:47:51。
- `python local/field/20261116-503-41/prepare-transfer.py open-window` 开启一小时注册窗口，截止北京时间 2026-11-16 20:50:13。现场申请尚待客户端首次启动，批准后关闭窗口。窗口过期可从管理端重开，原注册材料在上述有效期内继续可用。
- `python local/linux-deployment/check-field.py local/field/20261116-503-41/server-before-bundle.json` 确认 wccs/nginx/postgresql 均 active/enabled，release `da43fab-prefield-1113`、schema 12，可信 HTTPS 通过。服务器证书到期北京时间 2026-12-13 12:28:25，CA 到期 2027-02-11 12:28:25。

本次变更为单机交付包装及文档，Rust、数据库 schema 和前端源码沿用原交付。真实安装、恢复管理员创建与登录、Provider 和原生诊断仍待该实机执行。原通用包保持原摘要，统一镜像工作继续按下一节推进。Git 提交和推送等待明确要求。

## 统一镜像部署待补齐（2026-11-16）

用户明确常规部署通过统一镜像完成。此前让现场逐机复制配置的步骤增加了部署负担，后续入口按母机配置一次、镜像分发、首次启动自动初始化和管理端集中批量审核实现。逐机独立身份由客户端自动建立。

本次只读审查确认 `client-field.ps1` 仍要求显式目标和预置设备材料，`crates/wccs-server/src/device.rs` 的注册处理要求设备记录与注册摘要已存在。field.4 尚缺镜像封装、克隆后的身份初始化及配套批量注册入口，现有包与摘要保持原交付记录，完整镜像部署验收仍待完成。需覆盖母机凭据与恢复状态清理、重复 MachineGuid、断网启动、部分失败重试及复原软件下的身份保持。

## 首份现场采集回传（2026-11-16）

收到用户回传的安装前采集 ZIP，核对目标文件与导出证据的机器名、MachineGuid 一致。采集实际运行于 Windows 10 x64 EnterpriseS 21H2、19044.2130，Administrator 提升令牌，随包 PowerShell 7.6.9 正常运行，客户端包版本为 `0.3.0-field.20261113.4`。工具配置尚未下发，报告对应未配置状态。

卷枚举成功，只观察到 C 盘挂载及无盘符 FAT32 小分区，C 盘可用约 294.5 GiB。本机测试文件目录按既有 C 盘方案准备，D 盘与复原软件的现场条件继续核对。原始包、核对结果及填好服务器/CA 的逐机配置包保存在忽略目录 `local/field/`，上传内容按采集数据处理。

同日从开发机访问现有服务器 `/api/health`，可信 HTTPS 校验成功，返回 service=wccs、protocol_version=1。用户随后报告现场电脑可以通信，命令原始输出尚未回传，现场客户端的可信 HTTPS/WSS 仍随安装诊断核验。

日期为 2026-11-13，沿用 `feat/linux-deployment`，保留任务开始前的全部未提交改动。用户已结束虚拟化验证，本次系统级实际操作仅核对既有 Linux 测试服务器，开发机使用临时目录、只读采集与现有隔离 PostgreSQL。Windows 安装、机器 CA、SCM、账户、Credential Provider 和策略修改列入首次实机验收。

## 入口与实现

- `scripts/build-client.ps1` 固定 Rust 1.99.1、MSVC 14.44.35207、SDK 10.0.19041.0、PowerShell 7.6.9 x64。Cargo 使用 `--offline --locked`，客户端单独构建静态 CRT，链接器启用 `/Brepro`。Provider 复用原 C++ 构建入口并启用确定性链接。
- `scripts/package-client.py` 使用 Python 标准库校验 x64 PE、记录系统 DLL 导入、组件版本、源码及 Cargo.lock 摘要和完整逐文件清单。归档顺序、时间与权限固定，相同输入重打包可比较字节。源码重建受工具链和输入约束，当前保证已经固定输入的归档可重复。
- 包内 `client.cmd` 使用自带运行时调用 `client-field.ps1`。Verify/Collect 为现场起点，逐机目标由机器名和 MachineGuid 双重核对。SCM 原入口同步增加显式目标参数，登录入口移除旧 VM 名称白名单，现场名称允许下划线等 Windows 名称字符。
- Install 检查管理员令牌、Windows/x64、磁盘空间、保护目录、重解析点、服务路径、CA 摘要/用途/有效期和新逐机注册材料。分步骤持久记录，创建 Automatic/LocalSystem 服务，安装机器级 CA、服务器配置及成套文件。注册沿用既有管理端显式审核流程。
- Upgrade 使用同一受保护 bin 的暂存、预写日志、目录切换、读回校验和旧版本保留。失败自动尝试回退，进程中断后由 Rollback 继续。身份和账户状态独立保留。RollbackInstall 先恢复教学、撤销本安装 provider、移除匹配 SCM，保留数据，并提供显式 RemoveCa 撤回本次新增 CA。
- InitializeLogin/Recover/RemoveProvider 复用原生实现，由提升管理员启动受保护的固定动作 SYSTEM 一次性任务。每次任务具有独立操作号，回执必须匹配本次动作，孤立活动任务阻止并发系统操作。服务挂死时只有显式 ForceStop 才结束已经核对路径、PID 和 Own Process 的宿主。
- Diagnose 复用 inventory 与八步诊断，增加安装摘要、服务、机器 CA/HTTPS 叶证书、账户阶段、教学启用、策略日志及上次安装步骤。公共 ZIP 只包含白名单字段，原始采集与诊断留在 private 目录。工具路径由管理员从服务端导出完整快照供采集检查，普通账户实际权限继续由三项模板核验。

现场完整命令、执行身份、恢复判据、注册步骤和十三步首次实机表见 [现场操作说明](offline-client-field-guide.md)，同一说明随包以 `FIELD-GUIDE.md` 交付。

## 修复的失败路径

1. 原登录脚本只接受旧 VM 名称，现改为显式真实名称及 MachineGuid 核验。底层 SCM 操作也必须提供目标身份。
2. 服务首次安装中断后，重试会核对原逐机身份并保留已生成凭据/轮换和配置文件。
3. 原生恢复现在在文件访问和策略恢复前核验 SYSTEM。未安装 native 配置且没有历史 native 状态时仍能完成策略恢复，配置丢失但历史状态存在则报告失败。
4. 离线诊断队列达到 32 份时继续导出当前本机证据，队列保持有界；服务端 ACK 按实际发送结果记录。
5. Provider 撤回在账户恢复失败时仍尝试移除本安装的注册项，并返回恢复未完成状态，保留原生密码入口修复条件。
6. 新目录事务覆盖同版本内容冲突、版本兼容、文件占用、重命名间失败、进程中断和重复回退。SYSTEM 子步骤使用独立关联号，避免上一恢复回执被下一安装步骤复用。
7. 实际带工具配置的采集发现成功项省略 reason 字段会触发 PowerShell StrictMode，中断导出。已按字段存在性提取，并补充成功/缺失两类工具和合成敏感名称的完整包验证。

## 实际检查

当前日志统一位于 `artifacts/offline-client/`，实际交付集中在 `artifacts/offline-client/delivery/`。

最终包为 `wccs-client-0.3.0-field.20261113.4.zip`，大小 109,692,519 字节，约 104.61 MiB，940 个载荷文件加一份 manifest。SHA-256 为 `2052f28fa1ca97b97eccd03643a288976af7fdbffacb9d7e2fbfeef8638ef8c8`。同名 `.manifest.json` 与 `.sha256` 一并交付。目录中另含相同版本的 `FIELD-GUIDE.md`、公开测试 CA、`test-site.json` 和交付文件总摘要 `SHA256SUMS.txt`。

`test-site.json` 提供当前测试服务器地址、CA 文件名/摘要和证书有效期，用于填写 Collect 生成的逐机目标。公开 CA 文件 SHA-256 为 `870c376949d254f488488117c37236f00408b7e1ec67fd57ddd4b6b17d968279`。每机身份、注册令牌和恢复密码继续由现场独立初始化，包内文件名与内容核验排除了凭据文件。

实际完整构建命令如下。使用已安装的显式自包含运行时目录，构建日志为 `build-final.txt`。

```powershell
pwsh -NoProfile -File scripts/build-client.ps1 -Version 0.3.0-field.20261113.4 -PowerShellRuntime 'C:/Users/yichu/.cache/codex-runtimes/codex-primary-runtime/dependencies/native/powershell' -Output artifacts/offline-client/wccs-client-0.3.0-field.20261113.4.zip
```

| 命令或场景 | 实际结果 |
| --- | --- |
| `python -m unittest discover -s scripts/tests -v` | 20 项通过，含新包确定性、Windows 别名/路径拒绝、真实入口参数/错误机器拒绝、文件事务及脱敏回归 |
| `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` | Rust 构建、fmt、Clippy、84 项常规测试、原生 provider、PostgreSQL 14 项、前端 36 项与生产构建通过 |
| Linux `CARGO_BUILD_JOBS=4 python3 scripts/check-linux.py`，配置独立 wccs_test | 29 项常规、14 项 PostgreSQL 综合、release/fmt/Clippy 通过 |
| 真实 Windows 静态 CRT agent `--diagnose-test` 连接 Linux HTTPS | 配置、认证、HTTPS、HTTP 心跳、WSS、原生 IPC 六项成功，服务端 ACK=true；SCM/安装标为开发测试 unknown |
| 首次 Windows 组合检查 | 本地既有 PostgreSQL 集群停止导致 10061；启动 `local/m2/postgres` 后完整重跑通过 |
| 实际包解压后 `client.cmd -Action Verify` | 940 项摘要与字节数全部匹配；包内运行时成功启动；含空格目录的 CMD 入口通过 |
| `client.cmd -Action Collect -EvidenceContext development -ToolsConfiguration ... -OutputDirectory ...` | 真实只读采集、已存在工具/缺失工具、私有原始报告与公共 ZIP 通过；合成凭据写入配置版本/名称/路径后未出现在公共导出 |
| 使用相同参数再次执行完整 `build-client.ps1`，输出 `repro-release.zip` | ZIP 字节完全一致，两个 SHA-256 均为上述最终摘要；`repro-result.json` 和 `repro-build.txt` 保存结果 |
| `python scripts/check_repo.py`、PowerShell Parser、两项 `git diff --check` | 通过；任务收尾再次核验工作区和暂存区 |

Windows 测试数据库使用既有 `D:/PostgreSQL/18/bin/pg_ctl.exe` 在回环 55432 启动，完成测试后用 `-D E:/project/WCCS/local/m2/postgres -m fast stop` 恢复初始停止状态。没有遗留本轮 agent/session 进程。Linux 联调使用已有合成开发设备的逐机材料副本，保存在忽略目录 `local/offline-client-tests/`，运行类别为 local_test。完整包采集同样位于该忽略目录，运行类别为 development。

## Linux 保留状态

任务开始核对 WCCS、Nginx、PostgreSQL 均为 active/enabled，当前 release `da43fab-prefield-1113`，二进制 SHA-256 `8ebbbf9d9bd91c8ef688184962cc7bb7ac05c0433fd709f5fa5d4c929aecc44c`，schema 12。TLS 链及地址验证通过。服务器证书于北京时间 2026-12-13 12:28:25 到期，CA 于 2027-02-11 12:28:25 到期，连接材料继续保存在忽略目录。Linux 检查仅更新测试源码目录并使用隔离测试库。

收尾再次核对三个服务 active/enabled、release/schema、证书及管理员 API，保留 2 台设备、1 场比赛，注册窗口关闭，原 VM 设备保持 revoked。状态证据为 `linux-before.json`、`linux-final.json`、`linux-final-api.json`。公开连接元数据中的旧 release 字段已同步为当前值。

## 实机剩余验证与下一步

首次现场从 Collect 和教学账户基线开始，逐项验证机器 CA/ACL/SCM、安装中断与重复、注册审核、恢复管理员图形登录、普通比赛账户即时登录、等待/解除、三项模板、断线保留会话、系统任务超时/服务挂死恢复、provider 占用与回退、真实重启/复原软件、策略冲突与 WinRE 应急入口，最后恢复教学并带回脱敏 ZIP。

当前新增安装和一次性 SYSTEM 包装层的软件/文件回归通过，SCM、证书存储、真实账户和 LogonUI 修改仍需明确实机目标。长期 WSS 超时、多屏混合 DPI、长期显示、RDP/S4、进程树及实际复原软件继续保留原验收项。下一项可执行工作为携带完整包和逐机注册材料按现场表启动代表性机器测试，开发侧继续比赛脚本与 DOMjudge 闭环。Git 提交和推送等待用户明确要求。

清理本轮旧候选 ZIP 和解包目录的命令被自动审批拦截，工具仅返回 `blocked by policy`，删除命令未执行。旧候选包继续保存在 `artifacts/offline-client/` 和 `local/offline-client-tests/`；最终可复制交付集合为独立的 `delivery/` 目录，已逐项核对总摘要。

## 统一镜像接续（2026-11-16）

新增镜像准备包和母机 Upgrade/Seal/Diagnose/Recover 入口，复用现有安装、恢复、包摘要和诊断。克隆任务自动生成当前系统目标与独立注册材料，封装处理原安装目录中的逐机材料，旧公共支持包随镜像保存。详见 [镜像记录](image-deployment.md) 和 [现场说明](image-deployment-field-guide.md)。


## M3～M5 客户端交付（2026-11-17）

新增比赛策略、HTTPS 分发和归档清理恢复能力，复用离线退出控制、保留会话、重连需新操作的既有规则。最新软件回归与服务器 schema 18 部署见 [开发记录](m3-m5-implementation.md)。待目标与流程确定后，按 [现场说明](m3-m5-field-guide.md) 验证，复原镜像保持与真实重启继续待验。
