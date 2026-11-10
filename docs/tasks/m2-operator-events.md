# M2 操作员角色与全局事件

日期为 2026-11-10。Windows 开发机闭环已完成，由单个主任务在 `feat/operator-events` 完成。起点工作区干净，已核对 `1790537`、`57e258a`、`ae31656`，Natsume 本地副本为 `9b2dca59cf2cd4358c02bc051e318da44cda85e4`。

## 实施选择

复用固定 Rust/PostgreSQL、React/TypeScript/Vite、Argon2 和安全随机能力。参考 Operators 的稳定身份、最后管理员保护、凭据版本复核及 Events 的筛选、游标和会话响应隔离。WCCS 独立实现并增加机房范围。上游许可为 AGPL-3.0-or-later。

| 能力 | 管理员 | 操作员 | 只读查看员 |
| --- | --- | --- | --- |
| 设备、座位、已分配选手、结果查询 | 全场 | 授权范围 | 授权范围 |
| 设备启用、禁用、工具重查 | 全场 | 授权范围 | 拒绝 |
| 注册窗口、材料、审核、撤销、重新授权、凭据轮换 | 允许 | 拒绝 | 拒绝 |
| 机房/IP、地址核对、全局工具发布 | 允许 | 拒绝 | 拒绝 |
| 比赛创建、名单导入/修正/分配 | 允许 | 拒绝 | 拒绝 |
| 操作员管理 | 允许 | 拒绝 | 拒绝 |
| 事件与历史 | 全场 | 按事件发生时范围 | 按事件发生时范围 |

管理员固定全场。其他角色可选全场或显式机房集合。全场授权可查询未匹配设备、未分配名单和未知范围历史。限定机房身份只读取匹配到授权机房的设备及已分配选手，组合事件必须全部落在授权机房，未知/全局事件不可见。名单完整历史和全局工具目标需要全场查询权限。机房/IP 与名单写入会影响全场唯一性、匹配和完整快照，因此由管理员执行。

账号稳定 ID 随机生成，账号名去首尾空白、ASCII 大小写折叠，新增账号限制为 3～64 位 ASCII 字母、数字、点、横线和下划线。旧配置账号首次迁入保留原名与密码摘要，登录继续接受旧输入。启动只初始化一次，后续配置更改不覆盖持久账号。显示名、角色、范围、启用状态或密码变更递增版本，使已有会话失效。最后一个启用管理员不能被禁用或降级，自身修改遵守同一约束。

写入在配置头事务锁内复核会话版本、角色、范围、业务基准和请求去重。请求回执绑定稳定操作者，权限变化后须重新登录并依据当前授权重试，同请求不同内容拒绝。密码使用随机盐 Argon2 摘要，回执、事件和普通响应不包含明文凭据。

事件由已有审计、设备事件、工具任务和名单历史投影，保留稳定来源标识、请求关联及原业务入口。旧记录缺少操作者 ID/范围时保持未知，不使用当前关系补造。事件永久保留，本切片没有删除或自动过期入口。接口盘点、精确授权和事件规则见 [ADR 0009](../decisions/0009-operator-events.md)。

## 验收步骤

1. 版本 5 迁移、持久账号与事务授权，盘点全部 API。
2. 全局事件模型、范围过滤、稳定游标、详情与业务关联。
3. 管理员/操作员/查看员页面闭环与会话清理。
4. PostgreSQL 协议回归、真实浏览器和 Windows 检查入口。
5. 现有完整检查、自审、同步文档并形成本地提交。

## 启动与浏览器路径

复用独立 `local/m2/postgres` 测试集群和 `local/m2/server.json`，全部账号、设备和名单为合成数据。本轮无新增依赖。版本 5 升级前的开发库备份为忽略目录 `local/operators/before-v5.sql`。

```powershell
pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json
& './target/x86_64-pc-windows-msvc/release/wccs-server.exe' --config local/m2/server.json
& './target/x86_64-pc-windows-msvc/release/wccs-agent.exe' --identity local/roster/reinstalled/device.json --once
```

现有 8787 实例从 `local/operators/bin/wccs-server.exe` 运行，PID 保存在 `local/operators/server.pid` 和 `local/m2/server.pid`。更新前先核对 PID 与路径、停止并等待退出，再复制新构建。数据库启动沿用 [工具切片说明](m2-tool-configuration.md)。服务日志位于 `local/operators/server.out.log` 与 `server.err.log`。

原管理员可继续使用 `local/m2/admin-access.json`。浏览器验证管理员材料位于 `local/operators/browser-admin.json`，A 机房查看员的新登录材料位于 `local/operators/browser-viewer.json`。这些合成开发材料均被 Git 忽略。

1. 访问 `http://127.0.0.1:8787/#/operators`，管理员登录后点击“创建操作员”。
2. 设置账号、显示名、角色、启用状态及全场/指定机房，输入或生成初始密码，核对预览后确认保存。
3. 账号行“配置权限”支持修改显示名、角色/机房、禁用/启用及重置密码。保留基准版本，失败可重试原请求，冲突时关闭详情、重新加载并核对。
4. 保存结果显示请求编号，“查询变更事件”可查看原权限、新权限和重置标记。自身权限变更导致重新登录，随后按原请求查询事件。
5. 切换操作员，在设备详情启停或重查授权设备。查看员保留查询入口，写入控件禁用，越权 API 返回 403。
6. `#/events` 按时间、操作者稳定 ID、类型、设备、机房、比赛、请求和结果筛选。详情可查询关联事件并跳转设备、名单、配置或操作员页面。
7. 设备结果关联同一配置版本及请求。旧记录身份/机房未知时明确显示，限定机房账号无法读取这些记录。

## 实际验证记录

验证日期为 2026-11-10。Windows 开发机、PostgreSQL 18.4、固定 Rust 1.99.1/MSVC/SDK、Node.js 26.13.0/npm 11.20.0 和真实 Chromium 浏览器。数据库协议测试各使用随机隔离 schema。

| 类别 | 实际结果 |
| --- | --- |
| Rust 常规检查 | release 构建、rustfmt、Clippy `-D warnings`、40 项常规测试通过 |
| PostgreSQL HTTP | 5 项综合测试通过，包含原工具、机房/IP、生命周期、名单及新增操作员/事件 |
| 角色与会话 | 未登录、只读写入、全局 API 绕过、跨房目标、预览后降权、禁用/恢复、重置、最后管理员、自身降级、跨标签页会话绑定、并发权限编辑、同请求重放/不同内容拒绝 |
| 迁移与恢复 | v1→v5、v3→v5、v4→v5；历史账号文字保持、旧范围未知；重启后持久角色/凭据/请求恢复，配置改名不初始化第二个账号 |
| 事件 | 时间、人员、类型、设备/机房/比赛/请求/结果过滤，详情 404 范围保护，同秒排序、首屏水位、翻页插入旧时间记录、设备移房后历史范围保持、重复结果仅一条、脱敏 |
| 模拟故障 | SQL trigger 注入审计写入失败，账号、回执和成功事件整批回滚，恢复后相同请求成功 |
| 前端 | 类型、ESLint、Prettier、28 项契约/逻辑/组件测试与生产构建通过，含角色控件与旧会话迟到响应隔离 |
| Linux 交叉检查 | Windows 宿主的 GNU 目标 cargo check 通过 |
| 仓库 | 119 个文件检查、14 项 Python 回归及工作区/暂存区 diff 检查通过 |

完整命令日志为 `artifacts/m2-operator-final-check.txt`、`artifacts/m2-operator-linux-check.txt`。协议模拟使用受控数据库与合成设备观察，真实 Windows 结果另列下节。

### 真实浏览器与 Windows

- 管理员在浏览器创建 `browser-room-a`，角色操作员、范围仅合成机房 A，核对当前/新权限后保存，事件保留前后账号状态和请求编号。
- 操作员登录后只见 A 机房及当前唯一匹配的 synthetic-win-01，其他机房、排除设备与未分配选手被过滤。发起工具重查后，持续运行的真实 Windows 0.2.0 客户端回传 PowerShell 与合成脚本存在，submit 为 not_configured。
- 请求 `9cc001de-6c07-4095-ae7e-34e4141f0a92` 关联管理动作、下发和结果三条事件，原任务 `d38868a567dba2dc1863c7cf5e4dd96a7e133f8c2d84d81c98e8e6c1de901bc1` 在事件详情和设备历史一致。点击事件中的设备入口打开相同设备侧栏。
- 保留“禁用设备”未提交预览，通过另一个管理员 API 会话降权，浏览器轮询后退出并清除详情/预览。以查看员重登后正常查看工具结果，重查禁用，生命周期写入按钮隐藏。直接导航 `/api/operators` 返回角色拒绝。
- 实际服务停止/重启后旧登录失效，重新登录保留全部账号和角色。浏览器完成显示名修改、禁用、启用和密码重置，查看员以新密码成功登录，最终为启用的 A 机房查看员 v4。
- 使用 `local/roster/reinstalled/device.json` 执行真实 `wccs-agent --once`。首轮输出 credential_rotation_persisted，下一轮输出 config=4 和 result_accepted。浏览器核对 credential_rotated 设备上报及工具任务结果。该设备处于已排除占用，事件明确显示未知机房范围。
- 真实 Windows 日志为 `artifacts/m2-operator-windows.txt`，核对 JSON 为 `artifacts/m2-operator-windows-events.json` 与 `artifacts/m2-operator-windows-rotation-events.json`。复验脚本 `local/operators/verify-windows.ps1` 仅操作合成设备和独立身份目录。
- 桌面与 390×844 窄屏验证通过，窄屏文档宽度与 scrollWidth 均为 375px，表格在容器内滚动。修复新增导航挤出退出按钮的问题，导航独立滚动，身份及退出入口保持可见。当前页面控制台错误列表为空。
- 截图为 `artifacts/m2-operator-accounts.jpg`、`artifacts/m2-operator-session-expired.jpg`、`artifacts/m2-operator-mobile.jpg` 和 `artifacts/m2-operator-events-final.jpg`。

## 自审与环境缺口

全部管理路由已逐项盘点。身份检查、目标归属和基准复核在同一事务锁内，旧会话和旧权限不能执行新写入。请求记录绑定操作者，密码重试使用加盐摘要，范围检查先于去重结果返回。浏览器所有请求统一处理失效，隔离旧响应，跨标签页切换身份由绑定头拒绝。

新事件保存操作时机房，已有未知历史不会用当前绑定补齐。当前状态及原设备历史也过滤旧任务范围，事件详情、请求过滤和游标水位遵守同一授权。事件流记录已提交业务事实，被拒或回滚的请求通过 HTTP 返回原因，本切片未提供独立持久安全访问日志。

本机复查 WSL 仍未安装，未发现 Docker/Podman，SSH 配置不存在且用户未提供远端目标。Linux 原生运行需 Linux x86_64 与 PostgreSQL 实例；HTTPS 需证书、同源反向代理及客户端信任；跨 VLAN/NAT/ACL 需现场拓扑；Windows 10 普通账户、复原软件、身份目录 ACL 与镜像恢复需机房测试环境。这些项目尚未实测。

下一项建议实施 Windows 服务、WSS、鉴权 IPC 与等待界面批量控制。DOMjudge 同步/导出/自动登录及 compilecpp/submit 继续按 M3 对应切片实施，打印和拍照保持排除。

## 本地提交

- `6bba401` 实现版本 5 迁移、持久操作员、事务授权、事件投影和 PostgreSQL 回归。
- `c18a34d` 实现账号/事件页面、角色控件、会话隔离、关联入口和前端回归。
- 需求、架构、路线图、界面规范、验证方案及 ADR 随文档交接提交保存。

最终完整 check-m2 退出码为 0，GNU 交叉检查、119 文件仓库检查、14 项 Python 回归和逐次暂存区检查通过。运行实例使用最终发布副本，服务错误日志为空，既有持续 Windows 检查进程继续运行。
