# M2 比赛账户、即时登录与断线撤回

2026-11-16 首台物理机普通比赛账户登录通过。首轮缺少工作目录导致检查失败并恢复 teaching，现场补齐目录/权限后重新预览确认，普通账户权限、SID、工具访问和默认桌面回执通过，用户确认进入比赛账户。包装层漏建目录已修复。用户要求 Windows 首次登录欢迎和配置初始化提前在工作人员赛前准备阶段自动完成，批量准备与复原后的保持列入后续实现和验收。详见 [实机修复证据](offline-client.md#普通账户登录通过与工作目录修复2026-11-16)。

2026-11-16 后续实机 InitializeLogin/Start 与八项诊断通过，教学账户启用且无待恢复项，管理端原生账户快照确认 recovery_ready=true。已创建独立合成比赛及目标座位并完成包内 PowerShell 文件检查。当前 prepare_login 预览为 session_confirmation_required，普通比赛账户尚未创建，等待工作人员保存工作并确认允许结束教学会话后接续。证据见 [登录初始化记录](offline-client.md#首台登录初始化通过2026-11-16)。

2026-11-16 首台物理机完成离线合并包安装和八项联网/服务/IPC 诊断，设备注册在线。回传明确 `login_configured=false`，下一步使用包内 `03-prepare-login.cmd` 准备恢复管理员并验证其原生登录，再执行 `04-enable-login.cmd`。本轮实机的普通账户登录与教学恢复尚待结果，证据和注册窗口收尾见 [单机安装记录](offline-client.md#首台安装与联网通过2026-11-16)。

2026-11-13 离线客户端补充现场 InitializeLogin、Recover、RemoveProvider 和 RollbackInstall，旧测试机名称白名单已替换为实机双身份核验。原生恢复在文件和策略访问前要求 SYSTEM，恢复任务由提升管理员在本机启动，无旧 VM 运行器依赖。命令和剩余实机验证见 [现场说明](offline-client-field-guide.md)。

2026-11-13 后续独立 Win10 使用新 Linux 部署重新审核注册，机器级 CA、LocalSystem、Credential Provider 即时登录、SID/默认桌面确认及鉴权 IPC 已实测。保留旧账户所有权日志，修复旧服务器大序号阻止新鉴权连接登录的问题；断网和 Linux 整机重启后教学账户恢复、原会话及源文件保留，旧授权未重放。Windows 重启与显示环境的细分结果见 [原生联调记录](native-windows-linux.md)，不覆盖解锁、RDP 和机房镜像缺口。

2026-11-12 后续等待加固将 DisableTaskMgr 的持久原值与恢复加入同一离线和本机 Recover 入口。恢复工具先取得身份排他锁，策略失败时继续尝试恢复教学账户，最终明确返回失败。普通比赛账户与提升恢复管理员的实测权限边界、连续崩溃及重启结果见 [等待加固记录](m2-waiting-hardening.md)。

同轮已补验恢复管理员的真实图形登录、两个账户会话共存时跳过准备、教学 Administrator 的完整退出与比赛账户接续登录。未确认注销返回 session_confirmation_required，确认后旧管理员交互进程退出，普通比赛账户桌面回执确认。下文 2026-11-11 的待验证项按该补充更新，Credential Provider 解锁、RDP 和现场复原条件仍待验证。

## 范围与实现

2026-11-11，分支 feat/contest-login-offline。起点 814b4de，已核对 6733290、27b5b75、aa02c4d、814b4de 均在本地历史中，开始时工作区干净。默认单主任务完成。

- Rust 协议和后端复用等待控制的固定目标、事务授权、持久去重、设备代次、连接代次、选手/座位快照和事件链。迁移版本为 7。
- 浏览器入口 #/login 提供准备登录、明确确认教学会话退出、恢复教学、逐机状态、等待衔接和关联事件。准备计划固定现有服务端工具配置版本与路径。
- 文件模拟适配器使用独立 machine.json、journal.json、排他锁及一次性内存许可，覆盖部分失败与恢复。界面明确显示“模拟”。
- 原生测试适配器使用 NetUserAdd、SID/随机所有权、DPAPI、受保护本地管道、Credential Provider V2 和默认桌面的会话回执。DLL 的 COM 边界使用固定 MSVC/Windows SDK C++17，业务逻辑继续使用 Rust。
- 锁屏、多会话、RDP 和未知会话跳过。已有教学会话需要保存工作并明确确认退出。无人登录及已登录比赛账户分别走凭据提交和桌面核验。
- 失联恢复保留当前桌面、程序、账户及文件。服务启动先恢复，重连需要新的预览和确认。外部状态冲突阻止新启用。

## 本地操作

开发测试实例位于 local/contest-login，使用独立 login_browser schema 和 8789 端口。前台模拟入口如下，身份文件由管理注册流程产生。

```powershell
& ./target/x86_64-pc-windows-msvc/release/wccs-agent.exe --foreground --identity local/contest-login/device/device.json --session-exe local/contest-login/bin/wccs-session.exe --test-mode --simulate-login
```

先在工具配置页面配置完整路径，在比赛账户页选择比赛与座位，预览当前会话、账户及恢复条件。保存教学工作后按实际需要勾选会话退出确认，再提交准备登录。完成状态必须显示目标 SID 与桌面确认。进入等待使用等待页面，显式恢复教学使用比赛账户页。失败或重连后先重新预览。

原生安装使用 scripts/waiting-service.ps1 建立独立 SYSTEM 服务及受保护目录，再使用 scripts/login-test.ps1 安装测试 provider。当前脚本要求 SYSTEM、显式目标机器名、实际 MachineGuid、停止的匹配服务和受保护安装目录。原生配置放在 data/device.login-native.json，包含机器名/GUID、比赛账户名、教学账户名、恢复账户名/SID 和明确工作目录。恢复密码通过受保护的本地 UTF-16 文件交付，调用 --provision-login-native 后保存为 DPAPI，工作人员保留离线恢复材料。

脚本的 Recover 动作可在服务器关闭及服务停止时撤回本轮 flags。RemoveProvider 动作先恢复，再校验安装记录与注册表归属后撤销 WCCS provider 注册。比赛账户及文件继续留存。固定构建入口为 scripts/build-login-provider.ps1，已纳入 check-rust.ps1。

## 已记录的模拟与浏览器证据

真实浏览器连接 8789 实例，使用合成账号、选手和独立设备目录。已验证登录页面、固定预览、一台可执行与一台未绑定的部分目标、确认提交、接收/执行/模拟完成及等待控制衔接。

模拟登录请求 4b96ddc9-61cb-4f4b-85bf-79f7e7720fbe，命令 f746e71a2250aba1ed5dc5ad7cd235a401f7d48ada9be5d640cdadb480c60b34。随后实际启动开发模式等待窗口，等待请求 009027be-999b-4f3c-b86b-6e5308468c03。停止本轮 8789 服务后，实际窗口退出与文件模拟教学恢复合计测得 627ms，文件中的比赛桌面和账户保留，证据位于 artifacts/login-disconnect.json。此数值只对应本次前台窗口和文件模拟，真实账户恢复另行记录。

自动化覆盖账户同名/权限/工具条件、教学确认、锁屏/RDP/多会话跳过、每步进程退出、恢复冲突、恢复重试、一次性请求过期、断线、重复/旧命令、机器记录不一致和排他执行。PostgreSQL 第七项综合测试覆盖角色、范围、固定预览、重复请求、同编号异内容、连接更替、缺失真实完成证据拒绝及事件。原有六项综合回归继续执行。

恢复后重启服务，浏览器显示旧等待已取消、普通桌面与模拟教学已恢复，未自动恢复控制。通过浏览器另行预览并显式恢复，得到接收、执行中和成功，关联请求 3459e469-770f-4bf8-8bde-c102f146c361 的 login_accepted、login_executing、login_succeeded、login_observed 均可查询。截图保存在 artifacts/login-browser-recovery.jpg。

## Windows 10 测试环境

用户明确授权仅限 WCCS-Login-Test 虚拟机内的合成账户、登录组件、测试 Administrator、注销/切换会话及安装自动重启。全部文件位于 E:/project/WCCS/local/login-vm，64 GiB qcow2、6 GiB RAM、用户态 NAT，测试通信绑定宿主回环地址。已在 2026-11-11 18:08 关机建立 qcow2 内部快照 installed-baseline，再启用测试 Administrator 和安装登录组件。回滚须先关闭该虚拟机，再对 system.qcow2 执行 qemu-img snapshot -a installed-baseline。基线含独立恢复管理员和 SYSTEM 实验运行器，未安装 WCCS 登录组件，证据为 artifacts/login-vm-baseline.json。

镜像从 Microsoft 官方下载跳转取得 Windows 10 Enterprise Evaluation 22H2 19045.2006 英文 x64，文件大小 5,550,497,792 字节。QEMU Windows 构建为 20260805，SHA-512 与发布文件一致，解包后使用，未运行宿主安装器。初始安装使用 TCG，安装完成后已用 WHPX 冷启动运行。WHPX 在一次来宾重启时报告 Unexpected VP exit code 4 并暂停，退出该 QEMU 进程后冷启动恢复，记录在 artifacts/login-vm-whpx-restart.txt。宿主检查发现未提升令牌，硬件虚拟化接口可用。QEMU RTC 使用 base=localtime。

原生测试环境为 Windows 10 Enterprise Evaluation 22H2，机器 MachineGuid 为 8b041e9f-3bfc-432d-a620-7ca446dde702，SYSTEM 服务名 WCCS-Login-Test。合成比赛账户 SID 尾号 1003，独立恢复管理员尾号 1000，教学 Administrator 尾号 500。测试目录为 C:/Program Files/WCCS-Login-Test，编译器 C:/WCCS-Compiler/bin/g++.exe，工作目录 C:/WCCS-Work。

已通过真实系统核验并修复的事项如下。

- 服务安装 ACL 读取改为直接请求 SID，兼容英文 Windows 的 TrustedInstaller 与应用包权限项。
- 恢复管理员的交互登录令牌受到 UAC 筛选，成员核验改为读取关联的完整令牌，同时阻止管理员的筛选令牌冒充普通比赛账户。
- Credential Provider 的查询回调会同步触发一次性消费，原管道尚未回到监听状态时返回 ERROR_PIPE_BUSY。增加最多 200ms 的 WaitNamedPipe 等待后，已观察到真实 WccsContest 登录及首次用户配置初始化。
- 错误日期造成 Schannel 证书有效期校验失败，账户保持教学可用，校正测试机日期后 HTTPS 校验通过。随后将测试代理的 CA 证书与 CA=false 服务器叶证书分离，rustls 正常连接。证书校验全程开启。
- 已在普通比赛账户的交互会话运行 GNU C++ 编译和生成的程序，退出码 0，输出 native compiler ok，实际 SID 尾号 1003，管理员成员核验为 false。
- 已取得真实控制台 SID 和默认桌面会话回执，浏览器显示已完成。请求 269f83c5-57c0-4020-a0ab-1e41c2fb35c6，命令 c6df952627cca811df9524dbf4fd4aaabb4a1e070836e9adad78cdfbe132dfdb。截图为 artifacts/login-native-desktop.jpg。SYSTEM 默认管道所有者为 Administrators 的实测情况已修正为显式创建者用户 SID，普通会话仅获服务进程的受限查询权限。

真实系统剩余验证项如下。

- 教学 Administrator 已登录时的保存工作、网页确认和精确会话注销，以及工作人员从 Windows 密码界面登录恢复管理员。恢复凭据已通过 LogonUser 和完整管理员令牌核验，图形密码登录仍缺成功证据。
- RDP 和多会话的真实跳过路径。曾尝试 MSTSC，但凭据弹窗无法完成自动输入，未建立远程会话，测试 RDP 设置和端口转发已撤回。
- powercfg /a 显示 S3 受显示驱动限制，S4 休眠可用但本轮未执行。恢复过程中的进程退出逐步故障已有模拟覆盖，真实测试覆盖受控阶段崩溃及随后恢复。
- 安装重启已发生，完整的比赛授权跨系统重启、物理断电和机房复原软件测试仍待执行。系统盘与 D 盘真实不一致、多屏及现场镜像需要对应环境。

## 检查记录

最终 check-m2 通过 Rust release 构建、fmt、Clippy、60 项常规测试、7 项 PostgreSQL/HTTP/WSS 综合测试、33 项前端测试及 type/lint/format/build，同时完成 Credential Provider 的 W4/WX 构建。新增所有权标记相同而 SID 替换的拒绝回归、原生能力工具版本失效、管道创建者 SID 回归后已重跑通过。详细日志保存在 artifacts/login-final-check-m2.txt。仓库工作区检查和 14 项 Python 回归通过。Linux x86_64 交叉类型检查通过，当前无可用 Linux/WSL 发行版，未执行 Linux 原生构建或运行。

完整机房复原软件、D 盘/系统盘不一致、真实 RDP/多屏、200～300 台规模及 Linux 原生运行需要对应环境，继续按验证方案执行。


## 接口与测试工具来源

Credential Provider 的接口、默认提交与回调模型依据 Microsoft Windows SDK 文档及官方 V2 示例。阅读时固定官方示例仓库提交 434f6002bdf9cf9829406c3ff2b33387982d6168，本仓库实现独立的最小 COM、凭据序列化和定长本地协议。[微软 V2 示例](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/credential-provider/)

Windows 10 镜像使用 Microsoft 下载跳转，QEMU 使用官方项目链接的 Windows 构建，全部安装文件留在忽略的测试目录。QEMU 发布 SHA-512 为 0e4ff303ae0e7ef15b8dc963ea50a7daf6b706f1ceb31e07a228ef4e686272abd161507684b02976964a6112b6af0a7d49981de36d73ec6fcbaec6f418bec04c。[Microsoft 镜像入口](https://go.microsoft.com/fwlink/p/?LinkID=2208844&clcid=0x409&culture=en-us&country=US) [QEMU 下载说明](https://www.qemu.org/download/) [Windows 构建目录](https://qemu.weilnetz.de/w64/)


## 原生故障证据

所有计时来自来宾 Stopwatch，从请求本地测试代理注入故障之前开始，包含代理往返与 50ms 采样开销。单次测量用于记录当前测试环境行为。

| 实际故障 | 教学账户恢复 | 等待进程退出 | 保留及重连结果 |
| --- | --- | --- | --- |
| 已认证 WSS 明确关闭 | 449ms | 454ms | 比赛 SID、源文件及未保存窗口 PID 3636 保留，重连为 teaching |
| WSS 静默丢包且系统日期回拨一天 | 5079ms | 5081ms | 同上，恢复日期后重连保持 teaching |
| SYSTEM 服务进程被终止，SCM 自动重启 | 10093ms | 249ms | 原账户禁用在崩溃后暂留，SCM 重启后恢复；后台重新观察为 teaching，旧命令 cancelled |

原始结果分别位于 artifacts/login-native-disconnect.json、artifacts/login-native-silent-drop-clock-rollback.json、local/login-vm/results/native-service-restart.json。服务重启验收还发现无命令分支未刷新本地 control.json，导致文件留有旧实例和旧实际状态，现已补充持久化。真实后端观察保存为 artifacts/login-native-service-restart-observed.json。

外部修改测试 Administrator 的密码过期 flags 后触发断线，恢复进入 recovery_failed / recovery_conflict，账户保持禁用。将测试外部修改恢复到本轮预期值后，自动恢复启用。该结果验证条件恢复的冲突识别，读取与 NetUserSetInfo 写入之间的跨进程竞争仍受 ADR 0011 的边界约束。

补充真实系统结果如下。

- 从无人登录状态通过网页命令触发 Credential Provider，自动建立 SID 尾号 1003 的控制台默认桌面并取得程序回执，未手动唤醒登录界面。请求 57bd3550-33eb-416c-9571-1bb740d066db，证据 artifacts/login-native-unattended-login.json。
- 准备步骤 0 断线停止后续操作，教学账户一直可用。凭据已消费且步骤为 login_pending 时断线，1488ms 恢复教学账户，Windows 已建立的比赛会话和源文件保留，命令没有误报成功。证据 artifacts/login-native-pending-prepare.json、artifacts/login-native-pending-submit.json。
- 更改所有权标记、禁用比赛账户、拒绝编译器执行权限、替换为错误的 DPAPI 加密密码，分别触发原生拒绝或失败，教学账户可用。所有测试修改随后恢复。证据 artifacts/login-native-failure-cases.json。
- 实际 LockWorkStation 后预览显示 locked / unsupported_session，保持跳过。证据 artifacts/login-native-locked-preview.json。
- 服务停止时独立 Recover 工具用 986ms 恢复原来禁用的教学账户，保留未保存测试窗口，退出码 0。证据 local/login-vm/results/native-local-recovery-complete.json。
- 多次崩溃耗尽既有 SCM 策略的两次重启后，服务保持停止，教学限制暂留。随后通过本机工具恢复，并重建同名测试服务重置实验故障计数，确认 Running 和教学账户启用。重建中 sc.exe 的 PowerShell 参数引号失败，改用 New-Service 成功。此项明确了自动恢复失败时需要本机恢复入口的边界，证据 native-service-restart-6553a123.json、native-service-newservice-1111.json 均在 local/login-vm/results。
- Rust 客户端在来宾日期前移两年后实际返回 InvalidCertificate(ExpiredContext)，退出码 1，教学账户启用。finally 恢复日期并启动服务。证据 local/login-vm/results/native-rustls-wrongdate-final.json。
- 单独关闭本轮 8789 服务时，原生服务保持 Running，教学账户启用，无等待进程，未保存测试窗口和源文件保留。重启服务端后重新认证管理页面，设备保持 teaching。证据 local/login-vm/results/native-teaching-server-off-final.json。

等待窗口沿用既有普通全屏原型和退出控件，本轮验证原生跨用户启动、关闭及回执。生产环境的快捷键、覆盖窗口和多屏控制仍按等待界面现场矩阵验收。

## 最终浏览器验收与运行状态

最后一轮真实浏览器操作全部成功。准备登录请求 e69ab3a4-2054-4b44-82af-31b5a7288946，进入等待 817b73c4-3d4a-41e0-9302-df022b403035，结束等待 af2cf983-fdf4-4cc4-b5cf-324bf93d08a4，显式恢复教学 99e883bb-37ec-4add-850b-f5c089dad952。关联事件可查询 login_accepted、login_executing、login_succeeded 及 login_observed。页面截图为 artifacts/login-native-final-restored.png、artifacts/login-native-final-events.png。

19:24 的最终来宾观察确认教学 Administrator、WccsContest、WccsRecovery 均启用，当前比赛会话保留，未保存合成测试窗口仍存在，源文件 SHA-256 保持 23641ECA340DD0D3464CDDB3CE589463B3A0CEB7EEB42CA68D32CBCBC4A5A351，无等待进程，账户阶段 teaching。服务 WCCS-Login-Test 为 Running，来宾 PID 5756。完整观察为 local/login-vm/results/native-final-state-1111.json。

交付时保留以下本轮进程供复核，PID 为记录时数值，后续操作先核对路径与端口。

| 进程 | PID | 用途 |
| --- | --- | --- |
| wccs-server | 45820 | local/contest-login，127.0.0.1:8789，独立 login_browser schema |
| wccs-agent | 38096 | 前台文件模拟设备，教学待机 |
| qemu-system-x86_64 | 4144 | 唯一授权的 WCCS-Login-Test，6 GiB 内存 |
| python | 41644 | 回环 8870，受随机 bearer 保护的实验运行器文件/结果服务 |
| node | 34484 | 回环 8872，测试 HTTPS/WSS 代理，故障模式 normal |

既有 8788 服务及 PostgreSQL 55432 继续供原开发工作使用。虚拟机冷启动入口为 local/login-vm/start-whpx.ps1，只能在确认对应虚拟机进程已退出后执行。关机快照回滚会移除基线后的来宾修改，操作前先保存所需实验文件，再核对 system.qcow2 和 installed-baseline。恢复管理员的测试材料位于忽略的 local/login-vm/guest-access.json，需工作人员离线保管。

本机恢复在该虚拟机 SYSTEM 上下文执行以下命令。先停止匹配的 WCCS-Login-Test 服务，完成后检查 Administrator 启用和 journal 恢复状态，再启动服务。脚本同时验证机器名称、MachineGuid 和安装所有权。

```powershell
& C:/WCCS-Lab/pwsh/pwsh.exe -NoProfile -File C:/WCCS-Lab/login-test.ps1 -Action Recover -ExpectedMachineName WCCS-Login-Test -ExpectedMachineGuid 8b041e9f-3bfc-432d-a620-7ca446dde702 -InstallDirectory 'C:/Program Files/WCCS-Login-Test'
```

下一项先补齐恢复管理员原生密码登录与教学 Administrator 完整退出链，再在可用 S4、RDP/多会话和机房复原镜像上补验。当前切片交付范围为独立可恢复 Windows 10 测试实现，生产矩阵保留上述未验收项。

## 本地提交与最终复跑

- b1159b4 实现协议、后端、原生账户/登录/恢复、SYSTEM IPC 修正、Provider、安装工具和回归。
- 63e6ecd 实现管理入口、固定预览、教学退出确认、逐机状态、恢复和事件衔接及前端回归。
- 文档交接独立提交，包含需求、架构、ADR、验证、路线图和本记录。

交付前再次运行 check-m2 全部通过，日志 artifacts/login-delivery-check-m2.txt。Linux 交叉类型检查再次通过，日志 artifacts/login-delivery-linux-check.txt。npm ci 报告既有 esbuild postinstall 的 allowScripts 提示，类型检查、测试和生产构建均成功。仓库工作区、暂存区、diff 空白检查及 14 项 Python 回归再次通过。

## 统一镜像接续（2026-11-16）

赛前账户准备复用比赛账户管理页面的受控登录及明确会话结束确认。真实桌面成功后保存准备标记与工具路径，恢复教学不会清除这份封装依据。镜像导入复核本地账户、普通权限、恢复凭据、配置文件和工具访问。详见 [镜像记录](image-deployment.md) 和 [现场说明](image-deployment-field-guide.md)。


## 比赛策略与赛后恢复接入（2026-11-17）

准备登录可选禁止可移动存储与普通用户软件关机。策略先保存机器绑定恢复记录，教学恢复、离线恢复及镜像封装重用同一恢复入口。策略变更需要新的准备登录并沿用现有结束会话确认。软件关机权限作用于本机普通用户权限持有者，恢复时补回原授权。模拟回退与中断回归通过，503-41 原生效果待按 [单机说明](m3-m5-field-guide.md) 验证。
