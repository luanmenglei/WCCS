# 独立 Windows 10 与 Linux 原生联调

2026-11-16 首台物理 Windows 10 21H2 完成机器 CA、LocalSystem 安装、注册、八项诊断、普通比赛账户登录、等待/解除和三项维护模板，定向静默丢包后会话/未保存内容保留、重连旧指令失效。最终已回到教学会话，账户及策略无待恢复项。安装尚未固化到复原基线，原生重启保持和故障恢复矩阵继续待验。实机证据和已修复工作目录遗漏见 [本轮交接](offline-client.md#本轮最终诊断与未固化镜像交接2026-11-16)。

2026-11-13 后续现场工具已迁入仓库并随离线包交付。原 VM 路径和 dispatch 命令作为历史证据保留，当前实机改用 Collect 生成目标记录及机器名/MachineGuid 校验。Linux 服务与证书已重新核对，软件结果见 [离线客户端交付](offline-client.md)，原生剩余项按 [首次实机表](offline-client-field-guide.md) 执行。

2026-11-13 后续用户决定结束虚拟化验证。VM、磁盘、安装介质、QEMU 和实验服务已清理，虚拟机设备凭据已撤销。本文所列 local/login-vm、local/native-linux 和 local/machine-tools-vm 的小型证据路径现在位于 `local/environment/retired-vm-evidence-20261113.zip` 的同名条目中。清理后的工作安排见 [现场前加固记录](pre-field-hardening.md)。

验证日期：2026-11-13。沿用 `feat/linux-deployment`，保留进入任务时的部署配置和文档改动；未提交或推送。服务器地址、凭据、机器身份、完整 SID、原始回执和实验脚本均在忽略目录。

客户端构建基于 `da43fab5ecb902e9843384b6e0b1407170c08964` 加本轮未提交修复，固定 Rust 1.99.1 / `x86_64-pc-windows-msvc`；agent 0.3.0、session 0.2.0、inventory 0.1.2。实际部署二进制以文末摘要为准，不能只靠未递增的包版本判断本轮修复是否已安装。

## 环境与回滚边界

- Linux 使用现有 Ubuntu 24.04.2 / PostgreSQL 16 / Nginx / systemd 部署，release 为 `da43fab-linux1`。服务器二进制 SHA-256 为 `82994890643d57bd73663bdddefc2389684763ebd4d5e58e6be5f36c311f2b08`。
- 独立来宾为 Windows 10 Enterprise Evaluation 22H2、build 19045。使用现有 QEMU 11.0.93（`v11.1.0-rc3-12129-g50892b90e4`），WHPX、6 GiB、IDE 系统盘、e1000 用户态 NAT。系统盘卷标签、序列、机器名和 MachineGuid 已与旧登录实验记录核对。宿主 E 盘不是测试卷。
- 新覆盖层为 `local/native-linux/system.qcow2`，backing 为 `local/login-vm/system.qcow2`。原磁盘的 `installed-baseline` 和 `before-waiting-hardening-1112` 快照及此前失败覆盖层均保留。未回滚或改写原磁盘。
- 初次恢复完成系统更新后，热重启触发 WHPX `Unexpected VP exit code 4`。末轮分别验证默认双 CPU、单 CPU、双 CPU 且 `kernel-irqchip=off`：前两者在固件阶段暂停，第三者留在 SeaBIOS 且未进入系统。恢复原双 CPU 配置并冷启动成功；**无人干预 Windows 热重启未通过**。上游相同问题的替代配置未解决本机环境，不能把冷启动恢复算成热重启通过。[QEMU 问题记录](https://gitlab.com/qemu-project/qemu/-/issues/2042)
- 服务安装在来宾 `C:/Program Files/WCCS-Login-Test`，服务名 `WCCS-Login-Test`，自动启动、LocalSystem。成套更新 agent、session、Credential Provider，并保留既有账户所有权和 DPAPI 恢复材料；安装前备份到受限的 `before-linux-1113`。已加载 Provider DLL 通过同目录重命名旧文件后替换，未在宿主注册 Provider。
- 测试 CA 导入来宾 `LocalMachine/Root`，未向宿主机器证书库导入。叶证书到期为 **2026-12-13 12:28:25 +08:00**，CA 到期为 **2027-02-11 12:28:25 +08:00**。运行前后均使用此 CA 验证 HTTPS。

## 本轮发现并修复

1. 原生登录日志保留旧服务器 sequence 94，新 Linux 服务器较小的序号被误判为回退。现在在同一已鉴权连接内保持单调校验，新 instance/connection 可开始新的序列；Controller 仍先核验设备、代次、instance 和连接。模拟适配器共用同一判断，增加旧范围拒绝及新范围接受回归。未删除账户所有权日志来规避校验。
2. 缺少 Windows 10 可执行文件兼容性声明时，原生版本采集把 PowerShell `10.0.19041.3996` 报为 `6.2.19041.3996`，导致脚本运行依赖不匹配。使用现有 MSVC 链接器嵌入 `scripts/windows.manifest`，agent/inventory/session 共用构建脚本，无新增依赖。独立 CLI 与已声明版本的 Windows PowerShell 交叉校验，并在来宾观察到修复前后差异。[Windows 兼容性声明](https://learn.microsoft.com/en-us/windows/win32/sysinfo/targeting-your-application-at-windows-8-1)
3. 文件占用导致原子重命名失败时，目标文件回退成功，但遗留任务 `.tmp`。回退现同时清理该任务的临时文件，并保留路径、重解析点、类型和大小检查。新增真实文件句柄拒绝共享删除的 Windows 回归；来宾故障复测通过。修复前遗留的唯一临时文件在核对摘要后清理，失败证据保留。
4. 执行中撤回授权原先显示泛化系统失败；对写入前的身份/授权变化返回 `cancelled` 及明确原因。中途写入的回退结果仍按实际回退状态报告，不以取消掩盖回退失败。
5. 控制通道日志补充受限错误码，区分发送超时、接收超时和关闭，禁止任意错误文本泄露凭据。新增显式启用的原生 WSS 连续交换测试，默认不连接外部服务器。

## 真实页面与 Windows 行为

管理页面通过真实浏览器操作；故障矩阵通过同一鉴权管理 API 编排，文件及账户证据来自受机器身份约束的来宾 SYSTEM 运行器。没有用文件模拟适配器替代下表的原生行为。

| 场景 | 实际结果 |
| --- | --- |
| 注册及 TLS | 关闭公共注册窗口，单独审核 `login-native-01`；SYSTEM HTTPS、WSS 正常，旧开发设备排除座位绑定，避免 NAT 地址冲突 |
| 完整诊断 | 安装、SCM、服务器配置、设备认证、HTTP、WSS、Session 0→用户会话鉴权 IPC 八项成功；服务端 ACK=true |
| 普通账户准备与即时登录 | 管理页面完成，Credential Provider 从登录界面登录现有受控普通账户；目标 SID 与实际用户 SID 一致，默认桌面已确认 |
| 等待/解除/教学恢复 | 真实页面进入等待和原生全屏合成身份已确认；最新版末轮再次准备/进入，页面解除请求 `4042f1ea-b712-47c6-bda4-a91a80ad57ab` 与教学恢复请求 `defc56ae-d131-4eb7-b6c6-8512414f9a2a` 均成功。长时间黑屏/前台失败单列，不以短时成功覆盖 |
| 桌面准备 | 管理页面逐机成功；普通账户读取 GNU C++、工作目录 `.lnk` 目标及网站 `.url`，内容正确 |
| 工作目录 | 管理页面检查成功；普通账户实际写入、读回、移除测试文件，工具路径可访问 |
| 脚本部署 | 修复兼容性声明后页面成功；普通账户实际执行合成 v1、末轮 v2 `verify.ps1`，逐文件 SHA-256、manifest 摘要与 active.json 一致，脚本安装目录写入被拒绝。末轮同时核对计划任务退出码 0 与结果文件时间，避免使用旧结果 |
| 文件占用 | 保持真实目标句柄，改写前序文件后在占用目标失败；逐文件原摘要恢复，修复后无任务临时文件，回执 `rollback_complete` |
| 部分写入后服务中断 | 监测到新网站内容后强制结束 agent；磁盘日志实际为 `applying`。重启服务回退原文件并补报 `interrupted / rollback_complete` |
| 激活后 ACK 前中断 | 仅在 Linux 回环临时代理拦截终态维护 ACK，其余接口继续。实际 journal 为 `committed`、结果 succeeded、ack=false 时强制结束服务；重启及解除拦截后补报成功，文件摘要和修改时间未变 |
| 重复任务 | 对部分写入失败任务及已激活任务重放原 commit；服务端返回原回执，来宾文件摘要和修改时间均未变 |
| 执行中取消 | 服务器已看到 executing 后发出取消，原文件完整保留。修复后末轮请求 `e7e4e062-0e95-491e-94c0-5d7a8f7383c9` 回执 `cancelled / connection_lost`，步骤原因 `preparation_authorization_ended`；真实页面显示已停止、已请求取消 |
| 断网 | QMP 将该 VM 的 net0 断开 12 秒后恢复。教学账户恢复、等待策略日志移除、原 session 与合成未保存缓冲进程 PID 保留、源文件摘要不变；重连保持教学待机，旧授权未重放 |
| Linux 整机重启 | 14:00:54 执行 `sudo systemctl reboot`，14:01:17 WCCS 已自动 active；PostgreSQL、Nginx 同时恢复，数据库接受连接。boot_id 改变，旧 agent PID 和用户进程保留、限制撤回，重新登录管理页面后任务历史完整 |
| Linux 内核 | 重启前 `7.0.0-38-generic`，重启后 `7.0.0-42-generic`；本轮没有执行操作系统升级，启动了服务器已安装的内核 |
| Windows 系统重启 | 三种配置均实际执行 `shutdown /r /t 5 /f`，未能无人干预进入系统。人工结束对应 QEMU 并冷启动同一覆盖层后，SCM 自动运行、CA/源文件/v2 激活保留、教学恢复、旧授权未重放；只将这些冷启动恢复行为记为通过 |

脚本包为本轮合成 `native-test` v1/v2、config1/config2，许可字段 MIT；这只验证部署机制，**不代表 compilecpp/submit 或 DOMjudge 功能验收**。工作目录为来宾 C 盘专用目录；来宾 D 盘是光驱，本轮没有覆盖数据卷清盘和复原软件。

## 实际命令与证据

以下忽略目录脚本是本轮实际使用的编排记录，读取它们后再复跑；含状态前置条件和一次性名称，不应无条件重放。所有 dispatch 任务在执行脚本前校验来宾机器名与 MachineGuid。

```powershell
pwsh -NoProfile -File local/native-linux/start-vm.ps1
python -X utf8 local/login-vm/dispatch.py <unique-id> local/native-linux/resume-install.ps1
python -X utf8 local/login-vm/dispatch.py <unique-id> local/native-linux/diagnose.ps1
python -X utf8 local/native-linux/suite.py prepare_login enter leave restore_teaching
python -X utf8 local/native-linux/fault_files.py
python -X utf8 local/native-linux/fault-partial.py
python -X utf8 local/native-linux/fault-cancel.py
python -X utf8 local/native-linux/fault-ack-start.py
python -X utf8 local/native-linux/fault-ack-finish.py
python -X utf8 local/login-vm/dispatch.py <unique-id> local/native-linux/windows-reboot.ps1
python -X utf8 local/login-vm/dispatch.py <unique-id> local/native-linux/ordinary-access.ps1
pwsh -NoProfile -File scripts/check-rust.ps1
python scripts/check_repo.py
python -m unittest discover -s scripts/tests -v
git diff --check
git diff --cached --check
```

- `artifacts/native-linux/rust-final-check.txt`：固定 MSVC/SDK、发布构建、fmt、Clippy、Credential Provider 和 **83 项 Rust 常规测试通过**。14 项 PostgreSQL综合测试未在此入口执行；本轮未修改后端协议/数据库，沿用前一 Linux 部署记录的实际结果。
- Python 16 项回归通过，日志为 `artifacts/native-linux/python-check.txt`；最终工作区 204 文件检查、工作区与暂存区 diff 检查通过。没有执行 Git 暂存、提交或推送。
- 来宾实际执行 `wss-native.exe --ignored --nocapture`，使用停止 agent 后的独立测试身份文件环境变量 `WCCS_WSS_TEST_IDENTITY`，384 次 WSS 往返通过，耗时 82.59 秒。此测试不等价于长期稳定性验收。
- `local/login-vm/results/native-linux-*.json`：受保护原始诊断、机器身份、SID、服务、版本、源文件摘要及前后状态。`local/native-linux/` 保存每项管理请求、逐机结果、故障日志、完整文件摘要/修改时间比较和初始 Git 差异。
- 页面/屏幕证据：`artifacts/native-linux/login-confirmed.png`、`waiting.png`、`maintenance-results.png`、`final-maintenance-results.png`、`final-leave.png`、`server-reboot-recovered.png`（重启后重新登录并完成末轮教学恢复的页面）；黑屏和 WHPX 启动图另行保留。
- 三次热重启日志为 `local/native-linux/windows-warm-reboot.err`、`single-cpu-warm-reboot.err`、`software-irq-warm-reboot.err`，各配置启动脚本保留。`native-linux-after-windows-reboot` 和 `native-linux-after-single-cpu-reboot` 虽如此命名，实际结果在人工冷启动后取得；不得作为热重启通过证据。最终冷启动证据为 `local/login-vm/results/native-linux-final-cold-recovery.json`。
- Linux 内部证据：SSH 用户目录 `wccs-test/logs/native-linux/` 中的重启前后状态、无载荷 TCP 时序及 ACK 拦截记录。临时 8789 代理已经停止，Nginx 已从备份恢复并通过 `nginx -t`。

## 当前未通过项与边界

- 曾多次出现 `control_receive_timeout`。Linux 抓包显示服务器及时发送应答，客户端随后超时；独立 WSS 探针通过。长期业务运行的根因尚未闭环，不能标记 WSS 稳定性通过。
- 长时间无交互后 QMP 实屏黑屏，等待窗口回报 `waiting_focus_lost`；SYSTEM 和用户 IPC 仍工作。已记录来宾原 AC 显示/睡眠超时 600/1800 秒，并仅在来宾设为 0；一次唤醒请求没有恢复显示。冷启动后恢复可见等待界面，`artifacts/native-linux/recovered-wait.png`；长期无交互显示稳定性仍未通过，不能用 `desktop_confirmed` 替代屏幕证据。
- WHPX 三种配置热重启均未通过；默认双 CPU 冷启动恢复可以重现。测试 VM 的图形/电源/虚拟化问题与 WCCS 服务自启动分开记录，不把未定位的 WSS 超时归因于 WHPX。
- 本轮单 VM、单显示器、NAT、合成选手。物理多屏混合 DPI、RDP/S4、真实现场镜像/复原、数据卷清理、200～300 台控制续租、500 模拟客户端、300 座位 UI、真实 DOMjudge、完整竞赛脚本和进程树专项矩阵仍待验证。

## 联调结束时的历史状态

Linux PostgreSQL、Nginx、WCCS 保持 active / enabled，临时 ACK 代理已停止，8789 无监听。管理页面可重新登录、查询历史和控制设备。VM 保留默认双 CPU / WHPX 的新覆盖层，服务为 Automatic / LocalSystem，末轮已从真实页面恢复教学，原会话与源文件保留，脚本 `native-test` v2/config2 激活；维护配置已从故障模板恢复为 v8（三个入口及 v2 包）。来宾 AC 显示/睡眠超时保留为 0。基础 SYSTEM 实验运行器与宿主回环实验服务保留供续接；本轮普通账户访问/未保存进程的两个临时计划任务已移除。

最终来宾核对 `local/login-vm/results/native-linux-final-retained-state-3.json`：服务 Running / Auto / LocalSystem、教学账户启用、策略日志缺失、等待进程 0、临时计划任务 0、源文件摘要不变。核对时又捕获 `control_receive_timeout` 后的 `actual.state=failed / control_disconnected`，账户阶段仍为 teaching；控制连接会重试，但不能宣称保留状态持续在线。实验脚本最初返回扩展 PowerShell 字符串元数据超过回执上限，改为普通字符串后成功取得上述精简结果；无待执行 dispatch。

最终来宾二进制 SHA-256：

| 文件 | SHA-256 |
| --- | --- |
| wccs-agent.exe | `B3724DDF562EBD87FC0591C3836242B538337823186E1E5E8C9D3195B301DA67` |
| wccs-session.exe | `0A90F83DA0D4D3648D86549B53231708FD154B1DD763E34DF2699DF9B451ED07` |
| wccs-login-provider.dll | `DFC9C4D5B86B37E0B22A68F89BEB634EC33B857D3C3CF9857599104D76AEBC9C` |

用户随后决定清理虚拟化环境，Windows 热重启及长期无交互等待/WSS 转入首次实机复核。软件开发和自动化加固继续推进，执行顺序以路线图最新记录为准。
