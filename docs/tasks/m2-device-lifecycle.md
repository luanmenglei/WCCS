# M2 设备注册审核与凭据生命周期

日期为 2026-11-10。分支为 `feat/device-lifecycle`，基线 `b40b73b`，开始时工作区干净。已核对 `091b62b`、`8c67c55`、`9104bd4`、`e9b587a`、`b40b73b`，Natsume 本地参考基线为 `9b2dca59cf2cd4358c02bc051e318da44cda85e4`。

本轮实现 PostgreSQL 版本 3 迁移、注册窗口和审核、设备状态、可恢复凭据切换、管理页面及真实 Windows 入口。状态、兼容与部分失败选择见 [ADR 0007](../decisions/0007-device-lifecycle.md)。使用合成身份和独立数据库，沿用现有固定依赖。

## 实现与兼容

- 数据库从版本 1 / 2 事务迁移到 3，旧注册设备直接保留为 enabled，凭据摘要、机房位置、IP、全局配置和历史结果保留。历史未注册材料保留原有效期，新申请进入窗口策略。
- 逐机材料有效 24 小时，默认注册窗口关闭。开启后首次有效材料申请自动批准，关闭时待审核。窗口固定开放一小时，按持久绝对截止时间恢复。重新授权始终人工审核，已有待审核申请不随开窗自动批准。
- 已使用凭据摘要保留唯一归属，待审核申请锁定候选摘要。管理动作使用基准版本、请求编号及事务审计，重放相同请求返回原结果，内容不同或并发旧版本返回 409。
- 设备认证、任务领取、回传和状态变更共用事务锁，撤销提交之后旧凭据无法再写入。禁用保留占用和当前任务，停止新领取，已开始的只读检查可回传。撤销释放占用、取消未完成任务，已完成结果继续查询。
- 全局配置目标保留，禁用设备可接收新的等待配置并在启用后领取。重新授权不会自动重放旧任务，管理员显式重查。座位检查跳过 disabled，并复核实际预览。
- `wccs-agent` 0.2.0 使用原身份 JSON 和 credential 文本，兼容内嵌 device_token。已有 credential 优先，内嵌令牌仅在文件缺失时迁入。pending / completed 契约保留，取消任务回传得到 410 后清理 pending。
- `.rotation.json` 存放客户端预先持久化的候选凭据与请求编号，`.durable.tmp` 经 sync 与同目录 rename 发布。提交前中断重发同一候选，提交后响应丢失通过候选凭据心跳证明恢复，再替换 credential。旧凭据在切换提交时停止普通 API 访问，24 小时期限内只允许相同切换请求重放。候选文件丢失且服务端已切换时需管理员撤销和重新授权。
- 普通管理响应与历史接口返回状态、版本、关联号和结果，服务端仅存凭据摘要。管理材料在浏览器生成并下载，重新授权请求回执也只存材料摘要。完整 Bearer 身份目录复制仍代表同一设备，部署镜像需克隆后逐机建立身份。

## 启动与操作

沿用 `local/m2/postgres` 独立测试集群及 `local/m2/server.json`。本轮没有新增第三方依赖，Cargo.lock 只更新客户端自身版本。服务端生产资源仍位于 `web/dist`。

```powershell
pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json
& './target/x86_64-pc-windows-msvc/release/wccs-server.exe' --config local/m2/server.json
& './target/x86_64-pc-windows-msvc/release/wccs-agent.exe' --identity local/lifecycle/reinstalled/device.json --once
```

持续客户端运行时省略 `--once`。启动服务前检查当前实例，避免端口重复。本轮运行实例使用 `local/lifecycle/bin/wccs-server.exe` 的构建副本，PID 保存在 `local/lifecycle/server.pid`。重新构建后需停止该实例并更新副本，或从 target 启动新版本。管理员材料位于 `local/m2/admin-access.json`。

1. 访问 `http://127.0.0.1:8787/#/enrollment`。注册审核页开启 / 关闭窗口，展示当前截止时间与 pending / rejected / expired 筛选。
2. 从设备页新增设备，下载逐机材料到独立目录，Windows 入口先保存凭据再提交申请。关闭窗口时显示申请关联号并等待审核。
3. 管理员对照 Windows 输出核对稳定 ID 与申请关联号，在审核详情预览并批准或拒绝。材料泄露、丢失或申请错误时拒绝并重新授权。
4. 设备页按生命周期筛选，在详情逐设备预览禁用、启用、撤销或更新凭据。设备状态版本变化时取消当前确认、刷新再核对。失败可用原确认重试同一请求。
5. 凭据轮换需要 0.2.0 客户端。详情显示轮换请求和截止时间，成功后出现凭据切换完成事件。离线客户端在截止前上线即可继续，过期后管理员重新发起。
6. 重装或状态丢失时，先撤销原凭据，再重新授权并下载新材料到新身份目录。人工批准后到机房设置核对来源地址，按当前配置目标显式重新检查。
7. 详情中的生命周期及任务历史每页 100 条，保留取消原因与逐项结果，可加载更早记录。生命周期本轮提供逐设备操作，工具及座位批量任务继续使用目标预览和逐项结果。

重新授权提交成功但浏览器丢失新材料时，可拒绝尚未完成的申请，再重新授权生成新材料。管理员需现场核对设备与申请关联，IP 只参与座位匹配。

## 实际验证证据

验证日期为 2026-11-10，当前 Windows 开发机，PostgreSQL 18.4，固定 Rust 1.99.1 / MSVC / Windows SDK 及既有 Node.js / npm。自动化数据库测试各用随机独立 schema，浏览器使用原有合成机房和新增合成设备。

| 证据类型 | 实际执行与结果 |
| --- | --- |
| Rust 常规自动化 | 固定 release 构建、rustfmt、Clippy `-D warnings`、39 项测试通过 |
| PostgreSQL HTTP 协议自动化 | 3 项综合测试通过，包含旧工具配置、机房编辑 / IP / 并发及本轮生命周期 |
| 新增生命周期协议回归 | 未登录、CSRF、重复与不同内容重放、并发批准、非法转换、候选抢占、跨设备及历史凭据复用、禁用领取 / 回传、座位保留和撤销释放、轮换响应重试 / 过期、服务重启、拒绝 / 过期、重新授权、取消后迟到结果及历史脱敏 |
| Windows 可执行程序与模拟故障 | 真实 exe 在系统临时独立目录运行，模拟 HTTP 在轮换提交前 / 后丢失响应，退出重启后恢复同一候选；原 pending 结果重传回归通过 |
| 前端自动化 | 类型、ESLint、Prettier、20 项协议 / 组件 / 逻辑测试与生产构建通过；npm ci 成功 |
| Linux 交叉验证 | Windows 宿主执行 GNU 目标 cargo check 通过 |
| 真实浏览器与 Windows | 关闭窗口申请、人工批准、禁用、恢复、轮换、撤销、独立目录重装、再次批准与实际工具回传通过 |

日志位于忽略目录 `artifacts/lifecycle-check-m2.txt`、`artifacts/lifecycle-linux-check.txt`、`artifacts/lifecycle-windows.txt`。截图为 `artifacts/lifecycle-device-detail.png`，并发冲突截图为 `artifacts/lifecycle-version-conflict.png`。仓库与 Python 检查结果见最终提交记录。

### 真实闭环过程

- 浏览器创建设备 `lifecycle-win-01`，原始目录 `local/lifecycle/original`。第一次真实运行返回 registration_pending，管理员批准后开始心跳。
- 保留已有配置目标，新增该设备并发布工具配置 v3。禁用时客户端返回 disabled_paused，启用后执行原任务，PowerShell 和合成脚本存在，submit 为 not_configured。
- 浏览器发起轮换，客户端输出 credential_rotation_persisted，随后心跳成功。保存于忽略目录的切换前凭据探测得到 HTTP 401。
- 新建重查任务后撤销，未完成任务显示已取消，原身份程序退出码为 1。重新授权后原材料仍返回 401。
- `local/lifecycle/reinstalled` 模拟本地状态全部丢失，仅放置新材料，重新生成凭据并进入人工审核。批准后显式重查，实际结果成功回传。
- 浏览器开启窗口后重启服务，重新登录仍显示原截止时间，再显式关闭窗口。窗口和设备版本、历史记录持久恢复。
- 恢复设备地址要求核对，同回环 IP 的两台合成身份显示冲突。核对后排除新增设备占用，原有 synthetic-win-01 继续占用 A / 101，新增设备认证和工具检查仍可运行。
- 双标签页对同一设备操作，旧确认提交收到版本冲突，显示取消 / 刷新 / 重试路径，重新核对后恢复启用。浏览器控制台无错误。
- 旧身份目录 `local/m2/device.json` 使用新客户端运行通过，原 credential / completed 与机房布局兼容，pending 重传另由模拟响应丢失测试验证。浏览器核对 A 2×3、B 3×2 与原有工具路径，回归机房保存预览和地址核对入口。

## 自审与环境边界

认证与状态修改在同一数据库事务锁内，已提交撤销与轮换会阻止后来的旧凭据请求。轮换只接受管理员指定编号，历史摘要防止再次分配旧凭据，响应丢失通过预持久化候选恢复。请求去重记录与副作用同事务提交，恢复后不自动创建旧任务。

本轮修复撤销目标无法从配置草稿取消选择，以及重新授权后 desired_job 为空导致详情重查不可用的问题。全局发布允许暂停设备保留目标，显式重查仍要求 enabled。界面根据状态和配置范围显示可用操作。

实际 Linux 原生构建、HTTPS 证书与反向代理、跨 VLAN / NAT / ACL、Windows 10 机房复原镜像仍需要可运行环境和现场配置。本机可用证据为 Windows 服务进程、真实 PostgreSQL、浏览器与 Windows 检查入口，GNU 目标只有交叉类型检查。Windows 服务安装、WSS、等待界面鉴权 IPC、compilecpp / submit 包继续按后续切片实施。

下一项建议推进 M2 选手名单导入与座位分配，并准备 Linux / HTTPS 实例补验。凭据目录权限和镜像克隆恢复需用现场账户与复原软件补充验证。

## 最终检查与本地提交

- `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 最终退出码为 0，包含固定工具链 release / 格式 / Clippy / 39 项常规测试、3 项 PostgreSQL 综合测试、npm ci、前端类型 / lint / 格式 / 20 项测试和生产构建。
- `cargo check --locked -p wccs-server -p wccs-protocol --target x86_64-unknown-linux-gnu` 通过。
- `python scripts/check_repo.py` 通过 102 个文件，`python -m unittest discover -s scripts/tests -v` 的 14 项测试通过。工作区及逐次暂存区 diff 检查通过。
- `c6846ec` 实现服务端、版本 3 迁移、客户端 0.2.0、协议及故障回归。
- `c4a71fa` 实现注册审核与设备生命周期页面、历史查询和前端回归。
- 文档交接提交保存需求、架构、路线图、验证记录、界面规范和 ADR 更新。

最终开发实例已使用最新构建副本恢复。服务 PID 位于 `local/lifecycle/server.pid` 和 `local/m2/server.pid`，原合成客户端 PID 位于 `local/m2/agent.pid`，恢复客户端 PID 位于 `local/lifecycle/agent.pid`。两客户端连续心跳成功，服务与客户端错误日志为空。注册窗口已关闭，恢复设备处于 enabled、身份代次 2，其占用已排除以保留旧合成机在回环地址上的座位匹配。所有身份与日志位于忽略目录。
