# M2 Windows 服务、WSS 与等待界面批量控制

日期为 2026-11-11。分支为 `feat/waiting-control`，起点 `a469cc5`，工作区起点干净。核对 `6bba401`、`c18a34d`、`a469cc5` 和 Natsume 固定基线。单主任务完成，实现与恢复决定见 [ADR 0010](../decisions/0010-waiting-control.md)。

## 交付范围

版本 6 数据库迁移、固定目标和身份预览、进入/结束命令、逐阶段回执、最新实际状态、WSS 连接代次、授权失效、事件关联、Windows 服务宿主、受保护命名管道和真实会话窗口均已实现。沿用五包 Rust workspace、PostgreSQL、React/TypeScript/Vite、原工具检查和检查脚本。

管理页面 `#/waiting` 复用比赛选择、座位图、筛选、共享选择、表格与详情侧栏。管理员和授权操作员可操作，只读身份保留查询。名单、机房、设备状态或权限变化后需重新核对。操作默认有效 900 秒，可设置 30～3600 秒。重新核对读取最新观察，重试通过新预览下发新命令。

服务端重启取消旧等待授权，宿主/系统重启回到教学待机。短租约最长 15 秒，网络或 IPC 中断使界面退出。原成功回执和后续实际失败分别保留。比赛定时安排、DOMjudge、自动登录、选手命令脚本在 M3 实施。

## 独立开发实例

本轮使用既有 `local/m2/postgres` 集群中的独立 `waiting_browser` schema，服务端监听 `127.0.0.1:8788`。配置在忽略目录 `local/waiting-control/server.json`，合成身份在 `local/waiting-control/device/device.json`。旧 8787 实例和旧客户端独立保留。

```powershell
pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json
& './local/waiting-control/bin/wccs-server.exe' --config local/waiting-control/server.json
& './local/waiting-control/bin/wccs-agent.exe' --foreground --identity local/waiting-control/device/device.json --session-exe local/waiting-control/bin/wccs-session.exe --test-mode
```

服务端与前台宿主分别运行在终端，Ctrl+C 正常退出。后台专用实例的 PID 位于 `local/waiting-control/server.pid` 和 `agent.pid`。停止后台进程前核对 PID 对应完整可执行路径，再只结束该实例。旧实例 PID 文件和目录保持原样。

浏览器访问 `http://127.0.0.1:8788/#/waiting`，使用忽略目录 `local/m2/admin-access.json` 的合成开发登录材料。此实例迁移时独立初始化该管理员。

1. 顶部选择“合成等待控制验证赛”，在“等待控制”选择一个或多个座位。
2. 选择“进入等待界面”，核对有效期，点击“预览等待目标”。
3. 核对比赛版本、机房/座位、姓名/学校、目标设备、身份代次、用户会话和跳过原因，确认批量进入。
4. 逐机表分别显示等待接收、已接收、执行中和完成，以及当前实际显示状态。详情显示命令编号、请求编号、版本、到期与完成证据。
5. 选择“结束等待，恢复普通桌面”，重新预览并确认，核对完成和普通桌面。
6. 从详情进入“查询关联事件”，核对管理提交、设备接收、执行、实际观察和完成。
7. 本地点击退出按钮、Esc 或 Alt+F4 可退出测试窗口。页面随后显示实际失败，选择异常目标后重新预览可安全重试。

## Windows 服务安装与卸载

当前令牌实测为未提升管理员，`WCCS-M2-Waiting-Test` 实测未安装。本轮采用真实前台宿主完成端到端。以下 SCM 命令需提升 PowerShell，并使用指定独立目标。安装前脚本检查权限、服务配置、重解析点和父目录 ACL。

```powershell
pwsh -NoProfile -File scripts/waiting-service.ps1 -Action Install -Name WCCS-M2-Waiting-Test -InstallDirectory 'C:/Program Files/WCCS-M2-Waiting-Test' -Identity E:/project/WCCS/local/waiting-control/device/device.json
pwsh -NoProfile -File scripts/waiting-service.ps1 -Action Status -Name WCCS-M2-Waiting-Test -InstallDirectory 'C:/Program Files/WCCS-M2-Waiting-Test'
pwsh -NoProfile -File scripts/waiting-service.ps1 -Action Start -Name WCCS-M2-Waiting-Test -InstallDirectory 'C:/Program Files/WCCS-M2-Waiting-Test'
pwsh -NoProfile -File scripts/waiting-service.ps1 -Action Stop -Name WCCS-M2-Waiting-Test -InstallDirectory 'C:/Program Files/WCCS-M2-Waiting-Test'
pwsh -NoProfile -File scripts/waiting-service.ps1 -Action Uninstall -Name WCCS-M2-Waiting-Test -InstallDirectory 'C:/Program Files/WCCS-M2-Waiting-Test'
```

安装前停止使用同一设备身份的前台宿主，避免复制身份后竞争连接。安装复制既有 credential、pending、completed、rotation 文件并建立受保护数据目录。服务为自动启动但安装结束需显式启动，运行账号 LocalSystem。卸载只移除 SCM 注册，安装目录和身份保留供运维处理。

`device.host.log` 记录固定事件码与时间，单文件最多约 1 MiB 后重置。`device.control.json` 保存本宿主实例的命令、回执与观察，供诊断和同实例重试。服务重启不据此重新启用等待。旧工具结果持久文件继续由 HTTP worker 处理。实际服务的 Session 0、受保护用户进程和 SCM 停止/故障恢复需要后续提升权限验收。

## 自动化与真实证据

以下日志位于忽略目录 `artifacts/`，测试使用随机隔离 schema。浏览器和真实 Windows 证据使用上述独立实例。

| 类别 | 实际证据 |
| --- | --- |
| Rust | 固定 MSVC/SDK release、rustfmt、Clippy、48 项常规测试通过，日志 `waiting-m2-final.txt` |
| PostgreSQL / HTTP / WebSocket | 六项综合回归通过，含原五个业务切片和新等待控制，日志同上 |
| 新协议授权 | 未登录、只读拒绝、跨机房直接预览/提交、权限版本变化、原操作者回执、名单变化、设备禁用再启用、凭据轮换、身份代次变化 |
| 持久性与故障 | 同请求并发、不同内容复用、不同请求并发冲突、SQL trigger 注入事务回滚、服务端重启、过期、会话/宿主切换、原回执重放 |
| WebSocket | 两条真实本地 socket 的部分成功/失败、重复连接、旧连接写入拒绝、完成重复回执、错误完成证据、乱序阶段、64 KiB 超限关闭 |
| 本地 IPC | 真实 Windows 管道单元测试覆盖首实例独占、错误服务 PID、同用户假服务/假宿主镜像、远程管道名、消息边界、超限及断开 |
| 宿主状态 | 持久接收、无会话失败、单调截止时间、序号回退、完成后窗口丢失和同编号改内容回归 |
| 前端 | 类型、ESLint、Prettier、32 项测试及生产构建，含回执与当前实际状态区分 |
| Linux | Windows 上 GNU 目标交叉类型检查，日志 `waiting-linux-check.txt` |
| 仓库 | 134 文件文本/链接检查、14 项 Python 回归及工作区/暂存区 diff 检查通过 |

### 真实浏览器与会话程序

- 合成设备 `waiting-win-01` 使用独立身份目录，真实 HTTP 注册/心跳与 WSS 同时运行，用户会话键在预览中明确显示。本轮窗口目标为当前交互会话 16，使用可退出的普通测试窗口。
- 选择两个座位，预览显示林小禾/合成实验学校和陈小川/合成第二学校；第一台可执行，第二座位未匹配设备而跳过。
- 请求 `cde4c2ae-d86a-4074-a97c-ad79b0e1f5a6` 真实进入等待，命令 `851370f309da544dd74b7ae2aa32462de22a29547c814d388b1e11a0049c1893` 完成。原生窗口实际显示比赛、姓名、学校、座位，并观察到鼠标触碰聚合提示。
- 请求 `bb283c79-ed8c-45ec-98eb-7af459c171a9` 显式结束，命令 `63f079a5ce429d6382b6e3af2e10d3203018921cb88cc5e9cb07ff1952433e8b` 完成。浏览器显示普通桌面，窗口清单确认会话窗口退出。事件页按同请求显示管理、接收、执行、实际观察和完成六条关联记录。
- 宿主重启后原命令取消，重新预览才再次出现窗口。本地退出按钮使窗口退出，页面保留历史完成回执并显示当前失败，异常目标重新预览成功生成新命令。
- 在真实等待期间停止专用 8788 服务端，窗口退出。浏览器显示连接中断、实际状态未知及写入暂停。服务端恢复后旧等待授权取消，教学桌面保持。
- 持续宿主持有身份锁时，旧 `--identity ... --once` 返回 Windows 共享冲突 32，证明两入口不能在同一身份目录重复运行工具任务。
- 截图保存为 `waiting-browser-active.png`、`waiting-browser-events.png` 和最终桌面恢复截图。截图及日志含合成身份。

## 自审和环境依赖

自审补充了设备生命周期版本、精确座位占用复核、锁定状态查询、单调命令期限、停止子进程的实际确认，以及包含跳过座位的管理事件范围。Windows FFI 集中在服务和 IPC/原型平台模块，句柄、缓冲、回调和安全前提逐处说明。原 HTTP 工具 pending/轮换/结果回归保持通过。

SCM 实装、LocalSystem 到普通用户的跨会话启动、跨用户 ACL 与进程防注入需要提升权限及独立普通账户。本轮已有同用户真实管道测试和前台实际窗口证据，生产安全边界尚需上述条件核验。

Linux 原生、有效 HTTPS/WSS 证书与反向代理、跨 VLAN、Windows 10 机房镜像、复原软件、实际多屏/混合 DPI、锁定/注销/用户切换/RDP 均需要相应环境。服务和系统重启策略已通过协议/宿主实例测试，系统重启本轮按用户边界未执行。300/500 客户端容量仍属 M6。

下一项先在提升权限的独立 Windows 测试机执行 SCM、Session 0 和跨用户 IPC 验收，同时准备 Linux 与受信任证书补齐跨机 WSS。随后进入 M3 对应比赛流程与脚本切片。

### 最终运行证据补充

最终构建还实测了 30 秒有效期，请求 `232a9f8b-13cf-4247-8a7d-d3e991f1585d` 到期后页面显示过期及普通桌面，窗口清单确认退出。随后请求 `00762454-e013-4e7f-886c-d5b15035a096` 再次实际进入，显式结束请求 `8d899309-e301-47da-9689-caebacdaa940` 完成桌面恢复。

SCM Install 实际预检返回“requires an elevated PowerShell”，日志为 `artifacts/waiting-scm-preflight.txt`，专用服务状态查询仍为未安装。当前 WSL 查询提示需安装，未发现 Docker/Podman 和 SSH 配置，Linux 原生待提供环境。历史范围自审增加了设备跨机房移动后隐藏越权实时观察的回归，历史命令继续按原范围可查。

最终安全复验请求 `5784a315-3e01-4e3d-b306-47d296d3061c` 实际进入，进程 6028 的 `Get-ProcessMitigation -Id 6028` 查询显示 DisableExtensionPoints=ON、MicrosoftSignedOnly=ON，证据为 `artifacts/waiting-process-mitigations.json`。结束请求 `d1783389-ba9f-4e99-a937-90942105ff46` 显式恢复普通桌面。生产 SYSTEM 跨用户进程 DACL 继续需要提升权限环境验证。

## 交付时进程与检查状态

2026-11-11 最终 check-m2 退出码 0，48 项 Rust 常规测试、6 项 PostgreSQL / WebSocket 综合测试、32 项前端测试通过，构建、格式、Clippy、类型、lint 和生产打包通过。GNU 交叉检查、134 文件仓库检查和 14 项 Python 回归通过。

继续任务后重启独立开发数据库和最终构建的服务端，8788 服务端 PID 28268 保持运行，路径为 `local/waiting-control/bin/wccs-server.exe`。后台运行的前台宿主 PID 100520 已停止，全部 wccs-session 测试窗口已退出，最终客户端二进制已复制至同目录。`agent.pid` 保留最后测试 PID，重新启动时应更新。浏览器随后会将设备实际状态显示为离线未知，历史结束完成证据继续保留。继续任务时原有 8787 服务 PID 81392 和旧客户端 PID 69308、92888 已退出，本轮未重新启动这些旧实例。55432 独立开发 PostgreSQL 已恢复运行。

`WCCS-M2-Waiting-Test` 状态为未安装。最终浏览器恢复截图为 `artifacts/waiting-browser-final.png`，截取于停止测试宿主之前。422px 实际窄视口下文档宽度与 scrollWidth 均为 422px，表格在自身容器内滚动。

最终客户端额外验证了规范化 origin 到 WSS 的转换，兼容原 HTTP 入口接受的大小写形式，回归拒绝非回环明文、凭据型 URL、路径和查询。重新启动最终前台宿主后，数据库实测控制 connected=true；终端发送 Ctrl+C 后写入 `host_stopped`，进程退出。终端宿主因 Ctrl+C 返回 1，应用停止证据为上述日志和进程清单。

## 本地提交

- `5c5b56f` 实现版本 6、授权控制、WSS、SCM 宿主、IPC、会话接入与协议回归。
- `15ffc7a` 实现等待控制页面、固定预览、实际状态、恢复与事件入口及前端回归。
- `3722f4e` 修复规范化 origin 到 WSS 的转换。
- `bc3751e` 按比赛返回每台设备最新命令，覆盖单设备 501 条历史记录后仍可查询其他设备的回归；实际界面属于另一命令时明确显示。
- 本记录与相关文档交接形成独立本地提交。
