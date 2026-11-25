# Windows 首次实机操作说明

本说明随离线客户端包交付。适用 Windows 10 x64 1703+，既有原生证据来自 22H2。首次采集记录实际版本、更新修订和复原软件，较早镜像上的运行时及 API 兼容性逐机验收。包内固定 PowerShell 7.6.9 x64 与自包含 .NET 运行时，Rust 客户端和 Credential Provider 静态链接 CRT。运行机器无需 Rust、Python、Node.js 或 Visual Studio。PowerShell ZIP 部署方式及依赖来源见微软文档，运行时许可随 `runtime/LICENSE.txt`、`runtime/ThirdPartyNotices.txt` 交付。[PowerShell 安装文档](https://learn.microsoft.com/en-us/powershell/scripting/install/installing-powershell-on-windows) [.NET 支持系统](https://github.com/dotnet/core/blob/main/release-notes/10.0/supported-os.json)

本轮 Windows 部署流程和目标机器待定，503-41 已排除。以下为选用单机安装方式时的参考。

## 赴现场前

准备完整 ZIP、同名 `.sha256`、`.manifest.json`，独立核对传输后的 ZIP SHA-256。摘要覆盖包内每个文件，`manifest.json` 由外部 ZIP 摘要覆盖。包未做代码签名，现场以交付渠道记录的 ZIP 摘要核对来源。解包到本地普通目录，保留原包供恢复。示例统一在解包目录执行，路径按实际移动介质和机器填写。

```powershell
Get-FileHash -Algorithm SHA256 -LiteralPath 'C:\WCCS-Media\wccs-client.zip'
.\client.cmd -Action Verify
```

首次实机操作使用独立工作人员管理员的提升终端。安装入口按机器名与 MachineGuid 同时核验，所有系统操作限定到现场填写的目标。开发机只执行软件回归、只读采集或参数拒绝测试。包内没有机器身份、注册令牌、恢复密码、CA 私钥或服务器管理材料。CA 公共证书单独交付，核对其 SHA-256 后写入逐机目标文件。

服务器地址和端口以当次现场配置为准。当前内网测试入口 `https://10.16.37.123`，release `da43fab-prefield-1113`，schema 12。当前服务器证书于北京时间 2026-12-13 12:28:25 到期，测试 CA 于 2027-02-11 12:28:25 到期。超过有效期先续签服务器证书和更新信任材料，再执行联网步骤。

## 安装前采集与逐机初始化

在目标机运行下列命令。输出路径必须是尚不存在的本地目录。完整机器报告留在 `private/`，对外带回 `evidence-机器名-操作号.zip`。工具路径使用服务端导出的完整配置快照，未提供 `-ToolsConfiguration` 时显示未配置。采集器只读检查目录与工具，普通账户实际读写在维护模板中验证。

```powershell
.\client.cmd -Action Collect -OutputDirectory 'C:\WCCS-Evidence\before-01'
```

核对 `before-01/target.json` 的机器名和 MachineGuid 与实机一致，再填写 `server`、`ca_sha256`、安装目录和服务名。安装目录为受保护 `Program Files` 下独立一级目录，默认 `C:\Program Files\WCCS`。路径及子目录中存在重解析点时拒绝安装。克隆镜像后须重新采集并逐机初始化，已绑定 MachineGuid 的安装记录会拒绝在其他机器继续。

管理端“设备/注册审核”由管理员开启注册窗口，为本机生成独立设备 ID 和 64 位十六进制 enrollment_token。逐机身份 JSON 格式如下，保存到工作人员专用介质或受保护临时目录，勿放进共享安装包。

```json
{"server":"https://192.0.2.10","device_id":"lab-pc-001","enrollment_token":"逐机导出的64位十六进制令牌"}
```

安装器只接收上述三字段的新注册材料。正式设备凭据由客户端在本机首次运行时生成和持久化。开放注册窗口期间的首次有效材料申请自动批准；关闭窗口期间的申请进入待审核，重新授权始终需要人工审核。管理端核对设备 ID、申请关联号、来源 IP、座位及机器采集结果，对待审核申请执行批准，完成后关闭注册窗口。重装需撤销旧授权、使用新逐机材料和新独立安装目录，保留旧账户所有权和恢复日志供核对。

```powershell
.\client.cmd -Action Preflight -Target 'C:\WCCS-Evidence\before-01\target.json' -Identity 'C:\WCCS-Private\device.json' -CaCertificate 'C:\WCCS-Private\ca.crt'
.\client.cmd -Action Install -Target 'C:\WCCS-Evidence\before-01\target.json' -Identity 'C:\WCCS-Private\device.json' -CaCertificate 'C:\WCCS-Private\ca.crt'
.\client.cmd -Action Start -Target 'C:\WCCS-Evidence\before-01\target.json'
.\client.cmd -Action Diagnose -Target 'C:\WCCS-Evidence\before-01\target.json' -OutputDirectory 'C:\WCCS-Evidence\registered-01'
```

预检成功返回 `preflight_passed`。安装成功返回 `installed_service_stopped_run_start_then_diagnose`，服务为 Automatic/LocalSystem，data 目录仅 SYSTEM/Administrators 可访问，bin 目录普通用户只有读取执行权限。安装的机器级 CA 对 LocalSystem 生效。安装保持服务停止，Start 后进行注册审核，再重新 Diagnose。八项检查按 configuration、installation、server、authentication、http_heartbeat、websocket、service、session_ipc 核对，全部成功才完成联网和会话诊断。未登录用户时 IPC 可以未知，登录后再测。导出命令成功仅表示证据已经写出，逐项结果保存在 ZIP 内 JSON。

## 普通比赛账户与登录组件

先核对已有独立恢复管理员可通过 Windows 原生密码界面登录，记录其 SID，确认其密码由工作人员保管。教学账户和恢复管理员须不同，恢复管理员须为本地 Administrators 成员且已启用。若目标镜像尚无恢复管理员，由现场管理员在该明确目标上通过 Windows“计算机管理”创建并验证。下面只读命令可取得 SID。

```powershell
Get-CimInstance Win32_UserAccount -Filter 'LocalAccount=True' | Select-Object Name,SID,Disabled
```

逐机准备 `login.json`，`machine_guid`、`machine_name` 取自采集，`teaching_name` 使用现场实际教学账户名，`recovery_name` 和 SID 对应已验证的工作人员管理员，`account_name` 以 Wccs 开头且与前两者不同。工作目录使用本机明确测试目录，所有编程工具路径在服务端配置。

普通比赛账户登录前，`work_directory` 对应目录需存在并允许本地普通用户读写。单机材料 v2 的 `03-prepare-login.cmd` 和 `04-enable-login.cmd` 自动完成专用目录准备；使用下方通用 InitializeLogin 命令时，现场需先核对目录及权限。首次实机已发现缺少目录会在普通账户访问检查时中止并恢复教学，该故障及修正版见 [实机修复记录](offline-client.md#普通账户登录通过与工作目录修复2026-11-16)。

```json
{
  "machine_guid": "替换为实机MachineGuid",
  "machine_name": "LAB-PC-001",
  "account_name": "WccsContest",
  "teaching_name": "Administrator",
  "recovery_name": "WccsRecovery",
  "recovery_sid": "替换为实际恢复管理员SID",
  "work_directory": "C:\\WCCS-Work"
}
```

```powershell
.\client.cmd -Action InitializeLogin -Target 'C:\WCCS-Evidence\before-01\target.json' -LoginConfiguration 'C:\WCCS-Private\login.json'
.\client.cmd -Action Start -Target 'C:\WCCS-Evidence\before-01\target.json'
```

InitializeLogin 停止匹配服务、先恢复教学，再通过受保护的一次性 SYSTEM 计划任务校验恢复管理员凭据并安装 Credential Provider。密码在交互终端安全输入，仅以短期 UTF-16 文件进入 SYSTEM，完成或失败后清除该文件，持久副本通过既有 DPAPI 保存。一次性任务在结束后移除。系统断电留下的 `data/recovery.secret` 在下次 Recover 中清除，现场同时核对无残留任务。重复安装相同配置可重试，账户配置变化需要先恢复并核对所有权日志。

普通比赛账户由管理端“比赛账户”准备流程创建，核对实际 SID、普通权限、活动控制台和 `desktop_confirmed`。教学账户已有会话时，先让使用者保存工作，再在页面明确确认注销，首次实机记录该项。日常断线和教学恢复保留正在运行的会话、程序与文件。

## 升级、重试与回退

使用新的完整包解包到独立目录并先 Verify。版本不可变，同版本不同内容拒绝。Upgrade 先停止服务并恢复账户/策略，再将新 bin 暂存、核对摘要，预写升级日志，按目录重命名切换。旧 bin、旧清单和当前身份数据保留，新的程序需要重新 Start 并核对 Diagnose。

```powershell
.\client.cmd -Action Upgrade -Target 'C:\WCCS-Evidence\before-01\target.json'
.\client.cmd -Action Start -Target 'C:\WCCS-Evidence\before-01\target.json'
.\client.cmd -Action Diagnose -Target 'C:\WCCS-Evidence\before-01\target.json' -OutputDirectory 'C:\WCCS-Evidence\upgraded-01'
.\client.cmd -Action Rollback -Target 'C:\WCCS-Evidence\before-01\target.json'
```

Rollback 恢复最近一次更新前的 bin、包清单及部署版本，身份、账户日志、DPAPI 与现场文件持续保留，服务保持停止。重复 Rollback 核对旧摘要后返回。当前回退支持 state_schema=1 的成套版本，状态版本不兼容时拒绝升级。SCM 启动成功后仍需联网/会话诊断，失败时从同一完整包运行 Rollback。

首次安装需要撤回时，运行 `client.cmd -Action RollbackInstall -Target TARGET`。该入口先恢复账户和策略、撤销本安装拥有的 provider，再移除匹配服务，保留受保护数据与证据。如已确认机器 CA 也应撤回，增加 `-RemoveCa`，仅删除本次安装记录为新增的 CA。安装前已有的信任证书保持原有归属。撤回后可用相同材料再次 Install，注册身份继续沿用，登录组件需再次 InitializeLogin。

安装中断后重复同包、同目标和同身份路径的 Install，完成剩余阶段。安装成功后的重复 Install 核对二进制和服务。若目录仅创建到一半且尚无 `field-installation.json`，先采集目录所有者和内容，确认是本次空目录后由工作人员重命名隔离，再用原安装路径重新安装。已有历史测试安装缺少逐机记录时先使用原恢复入口恢复教学，再进行新目录安装和重新注册。

遇到 exe/DLL 文件占用，切换失败会尝试恢复旧 bin。若旧文件或回退目标也被占用，保留 `data/update.json` 与 backup/staging/failed 目录，执行 Diagnose，关闭对应的 WCCS 运维终端后重试 Rollback。Credential Provider 可能被 LogonUI 持有，先 RemoveProvider，确认教学登录可用并保存工作后安排现场重启，再重新升级、InitializeLogin。停止服务和恢复脚本均不主动注销选手或结束其程序。

## 本机教学恢复

所有命令由目标机独立工作人员管理员在提升终端执行，使用完整包或安装目录 `packages/版本/client.cmd`。服务停止后，一次性 SYSTEM 任务复用原生账户和策略恢复逻辑，核对 MachineGuid、SID、所有权与原字段。恢复成功后继续保持服务停止，便于工作人员核对教学状态。

| 故障 | 命令与成功判据 | 失败后的处置 |
| --- | --- | --- |
| 服务器离线或断网 | 等待单调超时撤回，再 Diagnose；等待窗口退出、教学账户启用、策略恢复、原程序和文件保留 | 执行 Recover，导出证据；联网恢复后在页面重新明确启用 |
| 服务崩溃或 SCM 重试耗尽 | `client.cmd -Action Recover -Target TARGET`；返回 teaching_recovered_service_stopped | 核对 data 中账户/策略日志和恢复管理员登录，修复具体冲突后重复 |
| 服务挂死 | 先 Recover，超时后显式 `client.cmd -Action Recover -Target TARGET -ForceStop` | 工具只结束已核对 SCM 路径、PID 和 Own Process 的 agent，随后恢复；进程身份不符时带回证据 |
| 登录组件异常 | `client.cmd -Action RemoveProvider -Target TARGET`，核对本包 CLSID 注册项消失、Windows 原生密码入口可用 | 即使账户恢复失败，也尝试撤销本安装拥有的 provider；原账户/策略失败仍需 Recover |
| 策略原值冲突或损坏 | Recover 后 Diagnose；policy_recovery_pending=false、教学账户启用、实际 Task Manager 策略与安装前记录一致 | 本机查看受保护 waiting-policy 日志，核对当前注册表类型、值、SID 与原值；仅修复已确认冲突后重试，保留无法确认的原始证据 |
| 服务程序损坏 | 从原完整包 Verify 后 Recover，使用包内受核对的独立 agent 恢复 | 有升级备份时执行 Rollback；首次安装可执行 RollbackInstall，再用原材料 Install、InitializeLogin 和 Diagnose，身份与文件保留 |

```powershell
.\client.cmd -Action Recover -Target 'C:\WCCS-Evidence\before-01\target.json'
.\client.cmd -Action Diagnose -Target 'C:\WCCS-Evidence\before-01\target.json' -OutputDirectory 'C:\WCCS-Evidence\recovered-01'
```

`data/field-result.json` 保存本次操作号、失败步骤和结果，`data/system-result.json` 保存 SYSTEM 子动作的独立回执。SYSTEM 失败的本地详情在受保护 `data/system-error.txt`，连同账户和策略日志供工作人员本机检查；公共 ZIP 仅导出白名单结果字段。

若登录界面异常导致无法打开终端，先使用 Windows 原生密码 provider 登录独立恢复管理员。若所有图形登录均受影响，现场从 Windows 恢复环境/信息中心镜像恢复入口处理本安装的 CLSID `{954886e2-4dc2-4e85-a962-2687f7658e0c}`，核对离线系统卷和注册表后撤销该项。该离线注册表应急路径需信息中心人员按实机系统卷执行，首次现场单独验证。当前会话和文件优先保留，重启或镜像恢复前先保存工作并导出证据。

Windows 恢复环境中先用 `diskpart` 的 `list volume` 只读核对目标 Windows 卷，退出 diskpart 后检查该卷 `Windows\System32\config\SOFTWARE` 和安装目录 `field-installation.json`。假定确认的离线系统卷为 `W:`，先备份 SOFTWARE hive，然后执行下列精确键操作。若目标卷不同，按已核对的盘符替换 `W:`。恢复环境中的临时挂载名称须未被使用。

```bat
reg load HKLM\WccsOffline W:\Windows\System32\config\SOFTWARE
reg query HKLM\WccsOffline\Microsoft\Cryptography /v MachineGuid
reg query HKLM\WccsOffline\Classes\CLSID\{954886e2-4dc2-4e85-a962-2687f7658e0c}\InprocServer32 /ve
```

MachineGuid 必须等于逐机目标记录，provider 路径必须等于安装记录中的 `bin\wccs-login-provider.dll`。核对通过后删除这两个精确键，再卸载 hive。返回 Windows 原生登录后用独立恢复管理员运行 Recover，确认教学账户和策略恢复。

```bat
reg delete "HKLM\WccsOffline\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{954886e2-4dc2-4e85-a962-2687f7658e0c}" /f
reg delete HKLM\WccsOffline\Classes\CLSID\{954886e2-4dc2-4e85-a962-2687f7658e0c}\InprocServer32 /f
reg unload HKLM\WccsOffline
```

## 首次实机逐步验收表

逐台记录包版本、ZIP 摘要、机器名、MachineGuid、步骤、实际结果、关联请求号和证据 ZIP。失败项保留失败状态，修复后使用新的输出目录复测。

| 顺序 | 执行动作 | 成功判据 |
| --- | --- | --- |
| 1 | Verify、Collect，记录系统/卷/网卡/复原软件/教学登录 | 摘要匹配；目标双身份与现场一致；原教学状态、文件基线可追溯 |
| 2 | Preflight、Install，两次重复 Install | CA 指纹正确；保护 ACL；Automatic/LocalSystem；重复执行保留身份且不创建重复服务 |
| 3 | Start、管理端批准注册、关闭注册窗口、Diagnose | 独立设备凭据，正确来源 IP/座位；HTTPS/WSS、注册、HTTP、SCM、会话 IPC 八项核对 |
| 4 | 验证恢复管理员原生图形登录，InitializeLogin，页面准备普通账户 | 教学会话按明确确认退出；普通比赛 SID、活动控制台、默认桌面回执一致 |
| 5 | 页面进入等待、触碰输入、F1、多屏/DPI、解除 | 身份信息正确；各屏幕实际可见；解除后策略恢复；长时无交互/WSS 超时单独记时 |
| 6 | 页面执行 prepare_desktop | 普通账户实际桌面 .lnk/.url 存在且可用，逐机任务成功 |
| 7 | 页面执行 check_workspace | 正确账户 SID、普通令牌，完整工具路径读/执行、工作目录读写通过 |
| 8 | 页面执行 deploy_scripts，发布新版本后重复任务 | 解释器版本、内容摘要、active.json 与请求版本一致，普通用户无法写脚本根目录；重复请求无重复副作用 |
| 9 | 停服务器连接/拔网线/静默丢包，随后恢复网络 | 单调超时撤回；教学启用；原会话、未保存程序和文件保留；旧授权失效 |
| 10 | Recover、重复 Recover、服务异常与 ForceStop 分支 | 策略/账户恢复、无残留一次性任务和密码文件、服务保持停止，教学可用 |
| 11 | 升级、同版本重试、文件占用、断电中断、Rollback | 整套摘要一致，当前数据保留，失败可按日志恢复旧 bin；跨版本状态兼容明确 |
| 12 | 保存工作，Windows 原生热重启及机房复原软件验证 | 自动服务、机器 CA、provider、身份、账户日志和卷恢复一致，旧命令无重放；系统盘/D 盘不一致单独记录 |
| 13 | 服务器离线，恢复教学、RemoveProvider 故障支路、最终 Diagnose | 教学图形登录、正常输入/程序使用；实际策略恢复；离场前带回脱敏 ZIP，并保管 private 原始材料 |

真实 DOMjudge、compilecpp/submit 命令合同与实际提交、P2P、清盘及全场容量按对应切片继续验收。此包包含客户端运维组件，三项模板依赖管理员提供的真实工具和竞赛脚本配置。现场脚本示例只用于验证部署，正式竞赛命令另行交付。

## 统一镜像接续（2026-11-16）

如选用统一镜像，需为选定母机制作匹配的材料包。旧单机材料和原始诊断的外部副本由工作人员在母机统一清理。详见 [镜像记录](image-deployment.md) 和 [现场说明](image-deployment-field-guide.md)。


## 更新后的单机测试

M3～M5 客户端、运行时、公开 CA、镜像入口、比赛脚本清单和现场说明合并交付。目标确定后，在选定代表机核对比赛脚本、策略及恢复矩阵，具体见 [单机现场说明](m3-m5-field-guide.md)。
