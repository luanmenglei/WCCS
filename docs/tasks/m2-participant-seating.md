# M2 选手名单导入与座位分配

日期为 2026-11-10。工作分支 `feat/participant-seating`，由单个主任务完成。开始时工作区干净，核对 `c6846ec`、`c4a71fa`、`c05d638`，Natsume 本地参考 HEAD 与指定基线一致。

## 交付与语义

版本 4 迁移、比赛上下文、UTF-8 TSV 导入与差异预览、资料修正、自动/手动分配、交换及解除、事务审计和历史查询均接入真实后端。名单页、座位图/列表与设备详情使用同一所选比赛和稳定座位关联。实现选择见 [ADR 0008](../decisions/0008-participant-seating.md)。本轮沿用锁定依赖，没有新增第三方依赖。

最小比赛只有 ID、名称、版本与创建时间，创建比赛后由管理员显式选场。每场名单和分配独立保留，稳定选手 ID 为单场参赛身份。重编号保留分配，缩减涉及已分配座位时需先在对应场次调整。禁用、撤销、重新授权及 IP 核对不会隐式修改选手计划。撤销后的设备失去占用，座位上的选手保持已分配。

### 导入格式

[合成 TSV 样例](../../config/participants.example.tsv) 可直接上传，也可点击“填入合成样例”。首行必须是以制表符分隔的 `code`、`name`、`school`。

| 字段 | 说明 |
| --- | --- |
| code | 场内唯一参赛标识，1～80 字符，区分大小写，保留前导零 |
| name | 姓名，1～100 字符，支持中文和内部空格 |
| school | 学校，1～200 字符，首版必填 |

UTF-8 可含 BOM，支持 LF/CRLF，首尾空白去除，空白行忽略。禁止控制字符、多行字段与引号转义。每批 1～500 行，上传文件最多 100000 字节，HTTP JSON 整体限制沿用 256 KiB。真实名单放在忽略的 `local/` 目录。

相同标识更新资料，缺失行和分配保留。逐行错误包含源文件行号，重复标识定位冲突来源。单选手修正可更改参赛标识并保留稳定 ID。自动分配顺序与跳过策略在页面展示，容量不足整批拒绝。提交失败保留原预览及请求编号，允许原请求重试；状态变化后点击“重新预览当前状态”核对新影响。

## 启动与浏览器路径

数据库仍使用 `local/m2/postgres` 的独立开发集群，连接配置和登录材料分别为 `local/m2/server.json`、`local/m2/admin-access.json`。启动数据库方法沿用 [M2 工具配置记录](m2-tool-configuration.md)。

```powershell
pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json
& './target/x86_64-pc-windows-msvc/release/wccs-server.exe' --config local/m2/server.json
```

若 8787 已有本轮实例，直接访问即可。更新前先核对 PID 对应路径再停止该实例。开发实例使用发布副本 `local/roster/bin/wccs-server.exe`，PID 保存于 `local/roster/server.pid` 和 `local/m2/server.pid`，日志在 `local/roster/`。

1. 打开 `http://127.0.0.1:8787/#/roster` 并登录。
2. 展开“创建比赛”，填比赛 ID/名称，预览后提交，再在顶部选择比赛。
3. 上传 TSV 或粘贴内容，点击“预览导入差异”，修正错误后确认提交。
4. 勾选机房，预览自动分配，核对每名选手位置和跳过原因后提交。
5. 在名单行点击“编辑与分配”，修正资料或选择座位，预览调整/交换/解除后提交。
6. 切换座位或设备页核对同场信息，回到名单页查看操作历史及持久请求编号。

Windows 验证入口继续使用原有发布程序与显式 identity 参数，所有本轮材料都位于独立目录。

```powershell
& './target/x86_64-pc-windows-msvc/release/wccs-agent.exe' --identity local/roster/reinstalled/device.json --once
```

## 验证记录

本轮使用 Windows 开发机、真实 PostgreSQL 18.4、Rust 固定 MSVC/SDK、既有 Node/npm 和 Chromium 管理浏览器。HTTP 回归创建随机独立 schema，浏览器只操作合成名单和机房。

| 类别 | 已执行证据 |
| --- | --- |
| Rust 与数据库自动化 | 40 项常规 Rust 测试、4 项 PostgreSQL HTTP 综合测试通过；包括 v1→v4 和 v3→v4 升级、历史配置与任务保留 |
| 名单/分配协议 | 未登录、Origin/CSRF、无效导入、中文空格、重复标识、重复内容、同请求重放/不同内容拒绝、缺失行保留、跨场隔离、稳定 ID、容量、重复占位、交换/解除、重编号与缩减拒绝 |
| 并发与故障模拟 | 同版本并发 200/409、机房变化/设备禁用导致预览失效、SQL trigger 注入写入中途失败后整体回滚、服务重建后重放原结果及审计计数保持 |
| 前端自动化 | 24 项契约/组件/逻辑测试通过，覆盖选场显示、规则说明、空态、断线禁用及预览字段 |
| 真实浏览器 | 合成比赛创建、重复标识与空姓名错误定位、导入差异、自动分配、交换、已分配资料修正、冲突座位禁止提交、解除及状态变化后重新预览 |
| 真实 Windows 入口 | roster-win-01 申请/批准、心跳、禁用暂停、恢复、工具检查、撤销旧凭据 401、独立目录重新授权、核对地址及显式重查；每步比较名单/分配快照保持相同 |

完整检查日志为 `artifacts/m2-roster-check.txt`，真实 Windows 证据为 `artifacts/m2-roster-windows.txt`。`local/roster/verify-windows.ps1` 为本机合成实例的复验脚本，setup、lifecycle、restore 三阶段分别记录真实 exe 输出和 API 状态。重新授权前后身份目录为 `local/roster/original`、`local/roster/reinstalled`。

真实工具结果保持 PowerShell 与合成脚本存在，submit 为 `not_configured`。浏览器已观察两台真实客户端同址冲突，随后核对占用并验证恢复设备详情的同场选手。生命周期演练后恢复 synthetic-win-01 的原占用，将新合成设备排除出 IP 匹配，工具配置保留新增测试目标及历史。

## 自审

- 参赛标识修改后再导入旧标识会创建独立 ID，回归避免覆盖原参赛记录。
- 编辑侧栏锁定打开时比赛版本，期间并发变化需重新加载/预览。
- 提交锁序沿用配置头，全场座位与设备观察和名单事务串行化。所有分配重写、资料更新、版本、请求结果与审计在同一事务内。
- 预览固定完整操作与目标，提交复算摘要。真实单次 Windows 客户端转离线时预览被拒绝，重新核对后提交成功。
- 解除、撤销设备和房间缩减各自使用明确行为，历史快照保留当时位置和资料。
- 原有机房/IP、地址核对、生命周期和工具配置三个综合测试全量回归。

## 环境缺口与下一项

Linux 原生构建运行仍需 Linux x86_64 测试机及 PostgreSQL；HTTPS 需证书、同源反向代理和客户端信任；跨 VLAN/NAT/ACL 需现场拓扑；Windows 10 镜像、普通账户、复原软件和身份目录权限需机房测试环境。Windows GNU 目标交叉类型检查单独记录，模拟故障与真实浏览器/Windows 入口证据分开保存。

下一项建议 M2 操作员角色与全局事件查询，之后实施 Windows 服务、WSS、鉴权 IPC 与批量等待界面。DOMjudge 同步/导出/自动登录和 compilecpp/submit 包继续按 M3 切片实施。

## 最终复验

- `scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 退出码 0，固定 Rust release、格式、Clippy、40 项常规测试、4 项真实 PostgreSQL HTTP 综合测试、npm ci 和前端检查通过。
- 后续 UI 自审补齐了座位列表的选手列及历史资料前后字段；已尝试提交但响应不确定时，即使轮询读到新版本仍允许重试原请求。`npm --prefix web run check` 再次通过，24 项前端测试包含座位图和列表一致性，日志为 `artifacts/m2-roster-web-final.txt`。
- Windows 宿主 `cargo check --locked -p wccs-server -p wccs-protocol --target x86_64-unknown-linux-gnu` 通过。实际 Linux 原生运行仍按上方条件补验。
- `python scripts/check_repo.py` 通过 110 个文件，`python -m unittest discover -s scripts/tests -v` 的 14 项测试通过，工作区及提交前暂存区 diff 检查通过。
- 浏览器完成两个场次，第一场 v7 保留两名已分配、一名已解除；第二场上传仓库 TSV 文件后在 B 机房独立分配，同参赛标识互不混用。双标签页修改产生旧预览禁用，重新预览入口恢复核对。
- 实际停止并重新启动发布服务后，浏览器旧会话失效，重新登录后两场名单、分配与操作历史均恢复，原 Windows 客户端继续心跳。
- 1440×1000 桌面与 390×844 窄屏验证导航、选场及座位显示；窄屏文档宽度 375px，无文档级横向溢出，表格在容器内滚动。浏览器控制台错误列表为空。
- 截图保存于 `artifacts/m2-roster-seats.png`、`artifacts/m2-roster-device.png`、`artifacts/m2-roster-stale.png`、`artifacts/m2-roster-mobile.png`。这些截图及运行数据均为合成开发证据。

## 本地提交

- `1790537` 为版本 4 迁移、名单/分配后端、机房缩减保护和协议回归。
- `57e258a` 为比赛选择、名单管理、差异预览、分配/交换、座位与设备展示及前端回归。
- 文档、合成 TSV 和受影响 ADR 随交接提交保存。
