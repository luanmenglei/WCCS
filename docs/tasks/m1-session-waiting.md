# M1 原生用户会话等待界面

日期为 2026-11-10。本切片交付 `wccs-session` 0.1.0，使用合成身份在当前 Windows 用户会话中独立运行。开发分支为 `feat/session-waiting`。

## 已有改动收口

先核对 ADR 0003、0004、候选清单、采集入口和历史报告兼容性。工具检查仅访问配置中的完整路径，脚本种类由配置指定，未配置项和路径缺失分别报告。compilecpp、submit 的独立脚本包交付继续按 M3 实施，服务端路径管理及配置下发按 M2 实施。

原有改动保留并提交为 `8da0e73`，提交说明为 `fix(inventory): honor server-managed tool paths and script kinds`。提交前通过发布构建、rustfmt、Clippy、23 项 Rust 测试、14 项 Python 测试、40 文件仓库检查及暂存区检查。

## 构建与运行

从仓库根目录使用 PowerShell 7 执行以下命令。

```powershell
pwsh -NoProfile -File scripts/check-rust.ps1
& './target/x86_64-pc-windows-msvc/release/wccs-session.exe'
```

发布产物位于 `target/x86_64-pc-windows-msvc/release/wccs-session.exe`。依赖沿用 Rust 1.99.1、windows 0.61.3、serde 1.0.228、serde_json 1.0.149、MSVC 14.44.35207 和 Windows SDK 10.0.19041.0。新增的 Windows API features 用于窗口、GDI 字体、DPI、显示器、会话通知和互斥量，Cargo.lock 中增加本地包条目。

原型使用 Per-Monitor V2 DPI，最低 API 条件为 Windows 10 1703 x64。机房镜像及 VC++ x64 运行库兼容性仍需现场确认。

每个显示器显示一个窗口，固定展示林小禾、示例实验学校、A-023 及等待状态。退出任意一个窗口会结束整个原型。可点击“退出开发测试”、按 Esc 或 Alt+F4，也可用 Tab 聚焦按钮后按 Enter 或空格。

如需独立记录目录，可运行以下命令。路径包含空格时使用 PowerShell 引号。

```powershell
& './target/x86_64-pc-windows-msvc/release/wccs-session.exe' --state-dir 'local/等待界面 验证'
& './target/x86_64-pc-windows-msvc/release/wccs-session.exe' --help
```

默认运行记录位于 `local/waiting/session-<会话号>/status.json`，属于忽略目录。程序退出码 0 表示正常退出，2 表示参数错误或本会话已有实例，1 表示平台、窗口或持久化失败。错误输出到 stderr。

必要时可依据 `status.json` 中的 `process_id`，核对进程路径后对该 PID 执行 `Stop-Process -Id <PID>`。下次使用相同 `--state-dir` 启动即可恢复窗口，并显示上次异常退出提示。

## 输入、状态与恢复

- 输入来源为本进程窗口消息队列中的按键按下、鼠标移动、按下和滚轮消息。自动重复按键和 Windows 合并后的鼠标消息按实际收到的消息计数。
- 采用单调时钟，首次触碰显示提示，冷却为 3 秒。冷却期间继续增加事件次数，提示截止时间保持固定。每屏共享本次运行的计数，界面刷新周期为 250 毫秒。
- 业务状态仅接收键盘或鼠标类别及经过时间。持久化字段限于版本、进程号、开始及经过时间、聚合次数、生命周期、显示器数量及 DPI。键值、字符、坐标和身份文本均无持久化字段。
- 同一个 Windows 会话使用 `Local` 命名互斥量，持有到窗口销毁和最终记录完成。重复启动返回 2；进程异常终止后，内核释放所有权，下一次启动可取得互斥量。
- 每次运行的计数从零开始。每秒更新最新聚合快照，先完成同目录临时文件并同步，再替换正式文件。异常终止时可能丢失最后约一秒的计数。再次启动读取 `clean_exit`，设置恢复提示，再发布本次运行快照。
- 正常退出写入 `clean_exit=true` 和 `visible=false`。损坏或未知版本的记录返回明确错误，可用新的 `--state-dir` 继续验证。写入失败会结束原型并向 stderr 报错，已经发布的快照仍可读取。
- 显示器配置变化时重新枚举并创建窗口，所有显示器共享原有业务状态。DPI 消息先应用系统建议矩形，再重新匹配显示器和缩放布局。布局按可用宽高适配，自动化覆盖 640×480 到 3840×2160、竖屏及 96～288 DPI。
- 首个窗口订阅当前会话的 WTS 通知。锁定和断开连接分别保存状态，任一成立时隐藏界面；解锁和连接分别解除对应状态并重建显示器窗口。会话注销或关机通知允许系统结束会话；若系统在最终落盘前结束进程，下一次启动按未正常退出处理。

当前原型的恢复入口为再次显式启动。服务启动用户会话进程、IPC 权限、远程控制及守护重启随后续服务切片实现。窗口采用普通非置顶 Win32 样式，系统任务切换保持可用，验证操作限定于本进程。

## FFI 与自审

所有 Windows 调用集中在 `crates/wccs-session/src/platform.rs`。纯业务模块禁止 unsafe。平台模块启用 `unsafe_op_in_unsafe_fn`，每处 unsafe 注释说明调用前提及所有权。

- UI 线程独占窗口、字体、定时器和互斥量。父窗口销毁其子控件，随后释放字体。WTS 注销先于窗口销毁，类注销后于全部窗口销毁。
- 窗口回调使用静态函数，只向主循环发送标量消息。显示器枚举回调同步借用独占向量，两类回调均拦截 Rust unwind。消息指针仅在对应同步回调期间读取。
- 自审修复字体替换的部分失败路径。先完成全部可失败的布局操作，再把新字体交给控件，避免错误返回时释放仍被控件使用的字体。
- 自审核对重复启动在文件访问前拒绝、快照部分写入、未知版本、同目录原子替换、异常进程释放互斥量和退出后的资源回收。
- CLI 参数检查拒绝缺值及把另一个选项作为目录值。实际输出路径被文件占用时退出 1，合成标记文件保持原内容。

API 行为核对依据 Microsoft 的 DPI、会话通知和命名互斥量文档。[WM_DPICHANGED](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged)、[WTSRegisterSessionNotification](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsregistersessionnotification)、[CreateMutexW](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw)

## 实际验证

当前开发机，Windows 内核版本 10.0.26300.9711，单显示器，窗口报告 DPI 144。使用发布 exe 和 computer-use 的真实窗口输入验证，本次操作范围为原型窗口和进程。

| 场景 | 实际结果 |
| --- | --- |
| 启动与身份 | 姓名、学校、座位、等待状态及退出按钮可见，原生控件可被 UI Automation 读取 |
| 键盘与鼠标 | 按键增加键盘计数，鼠标触碰出现 3 秒提示，冷却结束恢复等待文字 |
| 重复启动 | 第二实例退出 2，原窗口继续运行，第二次指定的目录未被创建 |
| 退出按钮 | 窗口及进程退出，记录正常结束 |
| Esc、Alt+F4 | 分别重新启动并实测，均结束进程并写正常结束记录 |
| Tab、Enter 和空格 | Tab 将焦点移至退出按钮，Enter 结束进程；最终构建另行实测 Tab 后空格退出 |
| 强制终止与恢复 | 核对 exe 路径后仅终止原型 PID；再次启动成功，界面显示异常恢复提示，`recovered_previous_run=true` |
| 正常重启 | 新实例计数归零，显示普通开发测试提示 |
| CLI 失败路径 | `--help` 退出 0，未知参数退出 2，记录路径被文件占用退出 1，原标记文件保持原内容 |

运行快照与验证副本保存在忽略目录 `local/waiting/` 和 `local/waiting-verification/`，包括 `button-exit.json`、`forced-exit.json`、`recovery-escape-exit.json`、`alt-f4-exit.json`、`tab-enter-exit.json`、`final-space-exit.json`。最终构建的退出记录核对 `clean_exit=true`、`visible=false`，对应进程已结束。实际窗口显示与各退出操作通过工具观察，以上目录保存聚合运行数据。

单实例等待状态采样间隔 48.6 秒，进程 CPU 累计时间增加 0.156 秒，工作集约 19.3 MiB，句柄由 223 到 221，线程由 6 到 5。采样 JSON 位于该目录的 `resources-start.json` 和 `resources-end.json`，长期运行与重复模式切换另行验收。`dumpbin /dependents` 实际检查到 `VCRUNTIME140.dll`、USER32、GDI32、WTSAPI32 等运行依赖，发布 exe 大小为 304640 字节。

## 自动化验证

`pwsh -NoProfile -File scripts/check-rust.ps1` 通过，覆盖固定工具链、C / Rust 最小程序运行、两个包的 release 构建、rustfmt、Clippy 和 32 项 Rust 测试。其中会话程序 9 项，覆盖冷却边界、计数溢出、会话锁定与断开叠加、布局尺度、参数、互斥量排他及释放、异常快照、部分写入和未知记录版本。

`python scripts/check_repo.py` 通过，共 45 个文件；`python -m unittest discover -s scripts/tests -v` 的 14 项测试通过。工作区 `git diff --check` 通过，暂存区另执行 `python scripts/check_repo.py --staged` 和 `git diff --cached --check`。几何尺度测试和会话状态单元测试使用合成输入，真实 Win32 多屏切换及系统会话通知留待下表条件下验收。

## 待现场验证与下一项

| 条件 | 待验证内容 |
| --- | --- |
| 双屏或更多显示器 | 每屏窗口、负坐标、不同主屏、热插拔、横竖屏切换、从任意屏退出 |
| 混合 DPI 和实际缩放切换 | 100% / 125% / 150% / 200% 字体及控件布局、DPI 消息、重建后的边界 |
| 可回滚 Windows 10 1703+ 机房镜像 | VC++ 运行库、普通账户启动、未提升及提升管理员、长期资源占用 |
| 独立测试账户或虚拟机、RDP 条件 | 锁定／解锁、快速用户切换、远程断开／重连、注销和关机通知 |
| 复原产品及测试镜像 | 重启后基础安装、运行记录保留、再次启动及登录方式验证 |

下一项建议推进 M2 的 Linux 服务、设备注册及心跳与管理 UI 基础闭环，先完成工具路径配置页面、鉴权、版本化下发和检查回传。机房镜像、登录与复原的 M1 现场验证并行排期，等待界面的批量远程控制按 M2 路线图接入。
