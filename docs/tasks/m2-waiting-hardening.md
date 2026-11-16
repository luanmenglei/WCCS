# 比赛等待界面防绕过加固与 Windows 10 验证

2026-11-16 后续物理机完成服务器端定向 TCP 443 静默丢包验证，等待退出、未保存记事本文字和 PowerShell 窗口保留。60 秒自动清理后连接恢复、旧指令取消、教学权限恢复且维持原比赛账户会话。临时防火墙表及清理定时任务已收尾，证据见 [断线记录](offline-client.md#服务器定向断线与重连通过2026-11-16)。

2026-11-16 首台物理机的普通账户等待与解除通过，用户确认身份信息、触碰提示及回桌面操作。首次实机安全软件弹出 DisableTaskMgr 修改提示，当轮连接断开并恢复教学权限，用户允许后新指令复测通过。此现象作为安全软件兼容项记录，完整证据见 [现场等待记录](offline-client.md#等待解除与三项维护通过2026-11-16)。

2026-11-13 现场恢复入口统一为离线包 Recover，使用机器名/MachineGuid 校验和固定动作 SYSTEM 一次性任务。服务挂死可显式 ForceStop，策略失败仍保留原值和账户恢复证据。下文 VM 命令为历史记录，当前逐机流程见 [现场说明](offline-client-field-guide.md)。

2026-11-13 同一隔离 VM 的新覆盖层已接入 Linux 原生服务，真实页面进入等待、解除与恢复教学通过，断网及 Linux 整机重启后策略及教学账户撤回通过。末轮最新版解除与教学恢复再次成功。长时间无交互后的黑屏/`waiting_focus_lost` 和 WSS 接收超时尚未定位，WHPX 三种配置的无人干预热重启均未通过；不能用短时成功或人工冷启动宣称长期运行与热重启通过。命令、实屏和保留状态见 [原生联调记录](native-windows-linux.md)。

日期为 2026-11-12。在 `feat/waiting-screen-design` 上继续用户已有的深色等待界面及两次现场窗口计划，单主任务实施，无提交或推送。源文件的视觉改版改动保留。架构决定见 [ADR 0012](../decisions/0012-waiting-hardening.md)。

## 实现

- 正式模式要求普通比赛账户已经受控，目标 SID 与会话一致。开发预览和 test 模式保留按钮、Esc、Alt+F4。
- 正式模式拒绝关闭窗口、退出按钮消息及外部 WM_QUIT，安装进程内低级键盘钩子，只聚合触碰类别。每屏置顶，250ms 周期核对前台并尝试恢复，保留深色身份与键鼠提示。
- 界面回执核验当前输入桌面和前台窗口。安全桌面或其他窗口取得前台时显示 unknown / waiting_focus_lost。前台恢复使用短暂输入队列关联，异常和挂死沿用宿主 IPC 超时撤回。
- SYSTEM 服务预写 DisableTaskMgr 的精确原值、类型、SID 和 MachineGuid，确认注册表持久化后启动窗口。正常结束先确认策略恢复再完成。服务重启、断线、会话变化和窗口异常均撤回本轮限制。
- 恢复支持未加载的用户配置单元，独立本机 Recover 与服务使用同一身份排他锁。恢复失败保留日志、继续恢复教学账户、维持控制状态连接并周期重试，后端预览阻止新的进入和准备登录。
- 协议新增固定 detail，React 页面显示中文失败说明，原 detail 继续显示。新增原值冲突、损坏日志、失败 detail 和真实 PostgreSQL 预览阻止回归。第三方依赖保持现有版本。

## 环境与证据分类

唯一系统级测试目标为 WCCS-Login-Test，MachineGuid `8b041e9f-3bfc-432d-a620-7ca446dde702`。Windows 10 Enterprise Evaluation 22H2 x64，WHPX / QEMU，6 GiB、双核、64 GiB qcow2。开始时虚拟机关机，核对原快照 `installed-baseline`，增加关机快照 `before-waiting-hardening-1112` 后启动。首次来宾报告确认教学账户启用、SYSTEM 服务正常及无交互用户。证据为 `local/login-vm/results/hardening-inventory-4.json`。

服务与会话程序实际路径在 `C:/Program Files/WCCS-Login-Test/bin`。首次复制曾落入安装目录根层，核对 SCM PathName 后修正，后续按实际 bin 路径和 SHA-256 验证。原生操作由既有受随机 bearer 保护的实验运行器执行，dispatch.py 每次核验机器名和 MachineGuid。管理服务使用本地 8789 / login_browser 独立 PostgreSQL schema，来宾经既有可信 HTTPS/WSS 测试代理连接。

系统键通过 QMP `send-key` 和虚拟键盘硬件路径注入来宾，组合键保持 150ms，每项保存来宾显示输出与管理状态。该证据覆盖虚拟 Windows 系统实际输入。普通账户和管理员权限探测使用目标账户的真实令牌和 Windows API；持久状态中断使用明确的日志注入，不能代替物理断电证据。主目录为 `artifacts/waiting-hardening/` 和 `local/login-vm/results/hardening-*.json`。

重跑入口使用 `local/login-vm/dispatch.py <唯一结果名> <来宾 ps1 路径>`、`hardening-keys.py`、`hardening-suite.py service`、`hardening-suite.py rounds`、`hardening-faults.py` 和 `hardening-physical.py`。这些实验脚本、配置与原始结果保留在忽略目录。开始下一次故障测试前重新核对机器身份、服务路径、控制状态与回滚点，沿用脚本中的身份核验。QMP 鼠标移动与点击间加入间隔，并以屏幕实际选中项核对，避免输入仍落在上一位置。

## 实测矩阵

| 场景与实际输入 | 预期 | 实际结果与证据 |
| --- | --- | --- |
| 管理页面选择原生测试比赛和座位，预览后确认进入 | 固定姓名、学校、座位，收到/执行/显示分开 | 实际原生深色窗口正确显示；最初旧 bin 演练后已更新并重新验证，最终页面闭环记录在下节 |
| Esc、Alt+F4、Alt+Tab、Alt+Esc、Ctrl+Esc、Win、Win+D、Win+Tab、Ctrl+Win+D/左右 | 保持等待及提示 | 12 项 QMP 输入后均为 waiting，截图 `keys-contact.png`，逐项 `keys.json` |
| Ctrl+Shift+Esc | 保持等待 | 同上，最后一项真实输入 |
| Ctrl+Alt+Del | Windows 安全桌面、正确观察状态 | Task Manager 入口隐藏；Lock、Switch user、Sign out、Change a password 保留，`ctrl-alt-delete.png`；修复输入桌面误报后 API 回传 waiting_focus_lost / unknown |
| 直接启动 Taskmgr | 显示禁用提示 | 普通用户真实启动显示 “Task Manager has been disabled by your administrator.”，`after-crash.png` 保存后台提示和保留的测试程序 |
| 等待前已运行 Taskmgr | 进程保持，普通用户不能结束等待 | PID 4372 在等待前启动并继续存在，进程结束权限单独实际拒绝；`hardening-preexisting-taskmgr.json` 与后续观察 |
| 普通比赛账户 Stop-Process、Stop-Service、修改策略 | 拒绝 | SID 尾号 1003，结束进程 Access denied、服务控制拒绝、策略修改拒绝，`hardening-security-1.json` |
| 提升恢复管理员结束等待进程 | 明确管理员能力边界，异常撤回 | SID 尾号 1000、管理员令牌为 true，结束进程成功，`hardening-admin-boundary-2.json` |
| SYSTEM 强制结束窗口 | 当前状态失败，撤回策略与账户，保留文件 | 单次 138ms 观察到教学恢复、无策略日志、无等待进程，未保存测试窗口和源文件保留，`hardening-window-crash.json` |
| 连续三次结束 SYSTEM 服务 | 前两次 SCM 恢复，耗尽后可本机恢复 | 第一次窗口 224ms / 策略 10179ms，第二次 120ms / 30175ms；第三次 225ms 退出窗口，38s 后服务仍停止、策略日志和禁用账户仍在。`hardening-service-crash-{1,2,3}-*.json` |
| SCM 重试耗尽后连续两次 Recover | 独立恢复且幂等 | 无服务器依赖，教学启用、策略日志移除，`hardening-local-recovery-eaec75.json` |
| 已认证 WSS 明确关闭 | 撤回本轮控制，保留会话与文件 | 账户 397ms、窗口 399ms，源文件和未保存窗口保留，重连 teaching，无旧等待，`hardening-fault-close-14b385.json` |
| WSS 静默丢包并回拨来宾日期一天 | 单调超时恢复 | 账户 5899ms、窗口 5711ms；finally 恢复日期和代理，重连 teaching，`hardening-fault-drop-99d409.json` |
| QMP 关闭虚拟网卡链路 10s | 恢复策略与账户，重连不重启等待 | 从来宾观察开始约 7.05s 恢复，断链在观察启动后约 2s 注入，源文件和窗口保留。`hardening-physical-link.json` 与重连观察 |
| 原 DisableTaskMgr 为 REG_BINARY 00 07 ff，进入后结束 | 精确恢复类型与字节 | 真实注册表恢复 Binary 和 `[0,7,255]`，日志移除，`hardening-raw-original-restored-64312c.json` |
| 受控时外部将策略改为 DWORD 2 | 保留冲突、回传失败、禁止新启用 | 保留日志，教学恢复；修复连接退出问题后浏览器持续显示恢复失败，预览 0 台可执行，`browser-recovery-failure.png` |
| 冲突修回本轮 DWORD 1 | 重试还原原值 | 自动还原 Binary `[0,7,255]` 并移除日志，`hardening-conflict-repaired-8bd5b6.json` |
| 拒绝 SYSTEM 对策略值的 SetValue 权限后进入 | 预写日志后的应用失败可回滚 | waiting_policy_apply_failed、教学恢复、日志移除。测试 ACL 的还原改用只请求 WRITE_DAC 的原生调用，最终原 SDDL 恢复，`hardening-partial-apply-observed-f81a08.json`、`hardening-undeny-native.json` |
| 注入“值已恢复、日志尚在”状态再启动服务 | 中断恢复可重复 | 启动移除日志，策略缺失、教学启用，`hardening-interrupted-restoration.json`；随后重复 Recover 成功 |
| 连续六轮进入/结束，最后恢复教学 | 无残留策略或进程 | `hardening-six-rounds-final-30eafb.json`，每轮控制请求保存 `hardening-control-*.json` |
| 普通账户投递 WM_CLOSE、WM_QUIT 0/1 | 本地消息不能关闭正式窗口 | 三项投递成功，实际 HWND 1835880 的进程仍存活，`hardening-local-messages-valid.json`。此前 FindWindow 空标题探测取得 HWND 0 的记录无效，已排除 |
| 普通账户请求 RunAs 提权 | 使用独立管理员凭据，安全桌面状态可观察 | UAC 显示 WccsRecovery 密码框，教学 Administrator 未列出；`uac.png`、`uac-state.json` 为 unknown / waiting_focus_lost，取消后返回等待 |
| Ctrl+Alt+Del 后真实鼠标点击 Lock | 锁定时撤回控制，保留会话 | 教学启用、策略缺失、日志移除，回传 no_session / locked，`locked.png`、`hardening-locked-verified.json` |
| 锁定后登录恢复管理员，保留比赛账户会话 | 观察多个会话并阻止新比赛准备 | session 1 为比赛账户断开，session 2 为恢复管理员活跃，原测试窗口和文件存在。prepare_login 即使确认注销仍返回 unsupported_session，`hardening-switched-sessions.json`、`hardening-multiple-session-preview.json` |
| SYSTEM 执行 tscon 回到原比赛会话、注销测试恢复管理员会话 | 特权本机恢复可返回桌面 | `hardening-system-return-session.json`。这是 SYSTEM 恢复操作；比赛账户锁定后的 Credential Provider 解锁仍未实现 |
| 停止实际 wccs-server 进程并重新启动 | 离线退出、重连隔离旧命令 | 从观察启动约 1.96s 策略、账户和窗口均恢复，测试窗口和文件保留；停止操作在观察启动约 2s 注入。`hardening-real-server-stop.json` 及 `hardening-server-reconnected-*.json` |
| 等待时禁用 SCM 重试、终止服务、真实关机再开机 | 未加载用户配置单元仍可恢复 | 首次暴露 AdjustTokenPrivileges 返回长度指针缺失，记录 hresult=800703e6，教学账户已恢复、策略日志保留且页面可见失败。修正后对同一份未加载配置单元恢复成功，无私人挂载及日志，`hardening-coldboot-recovery.json`、`hardening-unloaded-hive-fixed.json`；新增原生权限调用回归 |
| 冷启动额外配置 secondary-vga，普通会话请求扩展屏幕 | 记录虚拟机实际能力 | 主显示为 Microsoft Basic Display Adapter，1024×768、96 DPI；第二显卡 PnP Code 28 缺驱动，SetDisplayConfig 扩展返回 31，只枚举一屏。`hardening-display-dual-attempt.json`。真实双屏和混合 DPI 未获得可用驱动条件 |
| 开发预览实际 Esc、Alt+F4、鼠标点击退出按钮 | 保留便捷退出 | 三轮分别生成 clean_exit=true、visible=false、running=false，`hardening-preview-{esc,altf4,button}-result.json`，预览外观 `preview.png` |
| 修复后再次终止服务并真实关机/开机 | 启动自动恢复，无需登录比赛账户 | `hardening-coldboot-final.json` 在没有交互用户、HKU 未加载比赛 SID 时确认教学启用、journal=false、无私人挂载、源文件摘要一致、旧命令为空。SCM 10s/30s 重试配置已恢复 |
| 真实图形登录教学 Administrator，再准备比赛登录 | 需要明确注销确认，旧会话结束 | RID 500、管理员令牌 true、原测试进程 PID 8320。未确认返回 session_confirmation_required，确认后管理员交互进程结束、比赛 SID 1003 登录并确认桌面。`hardening-teaching-admin-session.json`、`hardening-admin-no-consent.json`、`hardening-admin-consent-login.json`、`hardening-teaching-admin-exited.json` |

## 最终管理页面闭环

使用真实管理页面与合成操作员 login-tester，选择 Windows 10 原生登录验证、对应唯一座位，预览固定合成选手小禾、独立测试学校及目标 SID。2026-11-12 的进入请求 `79f451e4-f3d7-44fa-979e-2b633cb84af1` 完成。详情显示等待界面已显示、ui_observed、屏幕 1、SID 尾号 1003 与桌面回执已确认。关联事件可见 waiting_commit、waiting_accepted、waiting_executing、waiting_observed 和 waiting_succeeded。证据为 `browser-enter-detail.png`、`browser-events.png`，原生窗口见最终 `keys-contact.png`。

从同一页面预览并确认统一结束，请求 `3b3dad34-5002-4640-944d-1cc86614daba`。详情实际观察为已完成、普通桌面、teaching_standby、屏幕 0。来宾核验策略缺失、日志移除及无等待进程，`hardening-browser-leave-guest.json`。随后执行独立恢复教学操作，继续完成预览与重启验证。恢复失败的页面及不可执行预览证据见 `browser-recovery-failure.png`。

上述计时为本机单次 Stopwatch 观察，包含采样、代理与调用开销。策略精确恢复与最终残留核对独立记录，不能从窗口退出时间推定账户或策略同样已恢复。

## 检查命令

```powershell
pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json
python scripts/check_repo.py
python -m unittest discover -s scripts/tests -v
git diff --check
git diff --cached --check
```

完整检查日志保存在 `local/waiting-hardening-final-check.log`，退出码 0。该入口覆盖固定 MSVC/SDK、Rust 发布构建、rustfmt、Clippy、65 项常规 Rust 测试、7 项真实 PostgreSQL/HTTP/WSS 综合测试，以及前端类型、lint、格式、34 项测试和生产构建。新增服务端预览回归在既有综合用例内执行。详情窗口增加当前原因后单独重跑 `npm --prefix web run check`，结果通过，日志 `local/waiting-hardening-final-web.log`。仓库检查通过，Python 14 项测试通过，两项 diff 检查通过。

## 恢复入口与最终交接

本机恢复沿用 `scripts/login-test.ps1 -Action Recover`。在指定虚拟机上停止匹配的服务，由已有 SYSTEM 运行器或独立恢复管理员建立的一次性 SYSTEM 任务执行。来宾已实际执行的命令如下，MachineGuid 与安装路径均经过核验。冲突或损坏日志保留时，先查 `data/device.host.log` 的操作名及 HRESULT，再核对策略当前值和受保护日志中的原值。外部值冲突需恢复到已记录的原值或本轮 DWORD 1，随后重试；无法确认来源时保留证据供运维处理。

```powershell
Stop-Service WCCS-Login-Test
& C:/WCCS-Lab/pwsh/pwsh.exe -NoProfile -File C:/WCCS-Lab/login-test.ps1 -Action Recover -ExpectedMachineName WCCS-Login-Test -ExpectedMachineGuid 8b041e9f-3bfc-432d-a620-7ca446dde702 -InstallDirectory 'C:/Program Files/WCCS-Login-Test'
Start-Service WCCS-Login-Test
```

最终发布构建的来宾 SHA-256 为 agent `8259F04549E0CF6B3D5B6E87712DFA2C24E448ED57FA5C9588726C6E1AC8A5CA`、session `32C840488F706B074C923FF724C60722032743F785EB72316D5F625C35B69658`，见 `hardening-final-binaries.json`。二进制来自本工作区固定工具链构建。最后一轮键盘矩阵在此二进制上重跑，12 项全部为 waiting。最终清理删除本轮 8 个 WCCS-Hardening 计划任务和首次复制的两个根目录副本，虚拟硬件恢复原单显卡配置。

最终来宾为 Administrator 图形桌面，session 3 活跃，服务 Running，教学账户启用，等待进程、恢复日志、私人配置单元挂载及本轮测试任务均为零。实时管理状态为 desktop / teaching，旧等待未重新启用。源文件 SHA-256 保持 `23641ECA340DD0D3464CDDB3CE589463B3A0CEB7EEB42CA68D32CBCBC4A5A351`。最终证据为 `hardening-final-teaching-desktop.json`、`hardening-final-live-status.json` 和 `final-desktop.png`。本地 control.json 保留最近持久化回执，当前账户与会话以实时管理状态和 Windows 会话枚举共同核验。

真实机房仍须核对复原软件、目标镜像及驱动、物理多屏混合 DPI、辅助技术与触摸、RDP、机房多会话组合、Linux 原生服务和批量规模。两次现场窗口计划保持。下一项可执行工作为真实 Linux 服务部署与可重复客户端安装/回退流程，把本轮恢复命令和证据采集整理进现场包。

普通比赛账户的常见快捷键、关闭与结束进程路径有相应防护。拥有独立管理员凭据或已有管理员令牌的进程仍可干预。Ctrl+Alt+Del 的锁定/切换、主动断网会按当前恢复模型退出控制。钩子超时静默移除、恶意本机程序和物理离线修改仍有边界，详见 ADR 0012。
