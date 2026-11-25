# M3～M5 连续开发记录

2026-11-25 更新：文中的 503-41 专用包、命令及原定现场步骤仅供追溯。本轮部署排除该机，具体流程和目标待定。

## 任务范围与起点

2026-11-16 用户要求完成 M3、M4、M5，并取消 P2P。文件分发采用服务器 HTTPS，保留分块校验、续传、限流、发布安排、逐机进度与补发。按正常管理 UI、API、持久状态、客户端、回归和浏览器验证逐切片推进。

Git 位于 `feat/image-deployment`、HEAD `7809210`，镜像切片及文档已有未提交改动全部保留。单主任务执行。开发机文件测试使用独立临时目录，原生账户、策略、重启和清理仅在明确指定的实机目标执行。

## 当前进度

- 已读需求、路线图、架构、开发规范、脚本 ADR、现有维护和比赛控制入口。
- 已同步取消 P2P 的需求、架构、路线图与项目指令。
- DOMjudge 真实联调材料原先缺失，用户授权在现有服务器安装并配置一个 judgehost。经官方版本核对，用户选择稳定版 9.0.2。
- 服务器为 Ubuntu 24.04.2、4 核 8 线程、15 GiB 内存，根卷可用约 171 GiB。WCCS/nginx/PostgreSQL 均 active，WCCS 发布为 `7809210-image-1116.4`。现有 cgroup v1 内核参数已生效，NTP 同步正常。
- DOMjudge 9.0.2 官方 tarball SHA-256 为 `f4c376e22fba85d064b247fe952ca3dfeb3d6b18f595ae367324f4ec9fa9effc`。开始安装独立 MariaDB/PHP-FPM 与 DOMjudge 到 `/opt/domjudge`，计划入口 `https://10.16.37.123:8443`，WCCS 使用原 443 入口。安装前配置保存在服务器 `/var/backups/wccs/domjudge-before-install`。

## 实施顺序

1. DOMjudge 服务与单 judgehost，真实正确/错误提交验证、访问材料与备份恢复入口。
2. M3 竞赛脚本契约和独立脚本包、服务端配置及真实提交，DOMjudge 账号/IP 核对与换机，在线绑定的定时安排与比赛控制。
3. M4 通用 HTTPS 文件分发、持久进度、内容校验、断点续传、发布与补发页面。
4. M5 USB/软件关机策略及恢复、事件告警、归档或明确跳过、明确目标卷/目录清理、重启及重新上线核对。
5. 受影响的 Rust、数据库、前端、Python 和 Linux 检查，浏览器端到端回归，文档与交付。实际实机验收项逐项记录。

当前软件交付已完成，M3、M4、M5 最终现场验收待补，最新状态见文末 2026-11-17 交付记录。以下按时间保留开发过程。

## 上游依据

版本选择以官方稳定发布页为准，安装按 9.0 文档与下载的 9.0.2 源码核对。文档在线分支可能包含后续补丁，行为以固定源码和实际测试为准。[发布页](https://www.domjudge.org/download)、[DOMserver 安装](https://www.domjudge.org/docs/manual/9.0/install-domserver.html)、[judgehost 安装](https://www.domjudge.org/docs/manual/9.0/install-judgehost.html)

## DOMjudge 部署验证（2026-11-17）

已部署 DOMjudge 9.0.2，入口为 `https://10.16.37.123:8443`，API 为 `/api/v4`。源码校验与编译通过，数据库使用独立 MariaDB，PHP 8.3 FPM 配置 8 个进程，判题进程以 domjudge 用户运行，单个 judgehost 绑定 CPU 2，运行账户为 domjudge-run-2。Ubuntu Noble chroot 使用清华镜像下载并验证发行签名，包含 G++、Python/PyPy 和 Java。服务及 create-cgroups 已启用开机启动。

安装中已修复 MariaDB 本地 socket 连接、默认数据包限制和示例题部分导入失败。boolfind 重新导入 10 个测试点后，三个示例题合计 15 个测试点。测试比赛为 demo，未配置正式比赛。内置 demo 登录账户已禁用，独立合成联调账号材料保存在忽略目录。

实际检查如下。

- HTTPS 证书验证通过，网页返回 200，API 报告版本 9.0.2。管理员 API 认证通过。
- 提交 1 为 AC，提交 2 为 WA，提交 3 为 TLE。均由实际 judgehost 编译执行，TLE 运行时间 5.992 秒。
- 数据库与配置备份为 `/var/backups/wccs/domjudge-20261116T161646Z`，文件仅 root 可读。备份恢复到独立数据库后，3 条提交、3 条判题及 15 个测试点数量一致，验证数据库已清理。
- 本地访问材料在 `local/domjudge/admin.json`、`ca.crt`，判题和恢复证据为 `judge-verification.json`、`backup-verification.json`。秘密材料未写入仓库文档。
- 浏览器打开后出现 DOMjudge 记分板标题，后续页面读取连续超时，因此浏览器登录与页面交互尚待补验。

尚待整机重启验证、比赛容量与判题时间稳定性测试。M3 的 IP 认证、换机旧授权撤销、脚本端到端及管理页面继续开发，M4 和 M5 尚未完成。

## 竞赛命令接入进度（2026-11-17）

新增 scripts/contest/ 下的 compilecpp.ps1、submit.ps1 和共享函数。PowerShell 7.6.9 沿用现有客户端运行时，参数通过独立 ArgumentList 传给编译器。scripts/contest-launch.ps1 固定一次调用的激活版本，核对源码与配置摘要。现有维护页面增加编译器工具 ID、编译参数、DOMjudge 地址、比赛和语言设置。旧配置继续可用，竞赛配置要求客户端报告 competition-v1。

普通账户脚本部署会生成 compilecpp.cmd、submit.cmd、比赛终端和桌面快捷方式。比赛终端从已准备的工作目录启动，当前终端 PATH 包含命令目录。版本目录包含维护配置编号，源码和配置全部写入并核对后才更新 active.json。

实际独立临时目录验证如下。

- GNU C++ 编译中文与空格目录中的程序成功，指定输出参数保留，编译失败清理旧 exe 与临时文件。
- PowerShell submit 以开发测试 IP 匹配合成 DOMjudge 用户，提交 4 获得真实 AC，随后清除临时 IP 绑定。
- 4 项脚本回归覆盖激活摘要篡改、参数转发、服务拒绝、响应丢失、服务器错误及重复调用。源文件字节摘要写入本地回执，响应未知时默认禁止重复发送。
- 完整 Windows Rust 检查通过，数据库新增配置/能力兼容回归，共 16 项通过。前端类型、lint、格式、37 项测试和生产构建通过。Python/PowerShell 共 27 项通过。Linux 原生检查通过，包含 29 项常规 Rust 和 16 项数据库回归。

DOMjudge 配置检查中的数据库连接数、日志文件大小、图片上传目录权限及密码哈希成本已修复，config/check 所有项通过。局部 PUT 配置的缺失字段重置问题已由安装后数据库备份恢复，改为完整 61 项快照更新并逐项核对。记录在 local/domjudge/ 下。

### 脚本包生成与现场调用

先从现有工具配置核对 PowerShell 文件版本，再生成一个新版本清单。

```powershell
python scripts/package-contest-scripts.py --source-directory scripts/contest --id wccs-contest --version v1 --configuration-version v1 --runtime-tool-id powershell --runtime-file-version 7.6.9.500 --source WCCS --license Project-internal --output artifacts/contest-scripts-v1.json
```

在“桌面与脚本维护配置”导入清单，启用 compilecpp 和 submit，填写本场编译器工具 ID、DOMjudge 地址、比赛 ID 和语言 ID，发布并对已登录普通比赛账户的目标执行脚本部署。选手打开桌面 WCCS 文件夹里的比赛终端。

```powershell
compilecpp main.cpp
compilecpp main.cpp -o solution.exe
submit A main.cpp
submit A main.cpp -Yes
```

默认成功回执可避免重复提交。结果未知时先核对 DOMjudge 网页上的提交记录，确需再次发送时显式使用 `-Resubmit`。源文件名遵循 DOMjudge 的 ASCII 文件名规则，所在目录可使用中文和空格。配置和脚本校验失败时，工作人员重新部署对应版本。

以上入口尚待更新客户端后在指定实机验证。DOMjudge 账号核对/换机管理页面、定时比赛安排、M4 和 M5 仍在开发范围内。

## DOMjudge 账户核对与换机通过（2026-11-17）

新增持久同步任务、账号核对预览、集中执行和失败后重新核对入口。账号绑定由选手稳定 ID 派生，每个账号对应一个队伍。IP 变化时先撤销所选旧绑定与网页会话，再逐一建立新绑定。随机密码仅发送给 DOMjudge，WCCS 不保存选手密码。同步范围冻结名单、座位、设备注册代次及操作员权限，执行中再次核对。远端响应未知与服务重启都要求工作人员重新预览。

已修复 DOMjudge CLICS ID 36 字符限制，新增长度和边界回归。重复同请求返回原任务，新请求发现绑定一致时跳过更新。Windows 数据库 17 项通过，前端 37 项通过，Linux 原生及 17 项数据库通过。后续 M4 检查扩展到 18 项数据库。

独立本地 WCCS 连接实际 DOMjudge 完成两名合成选手建号、互换座位、旧 Cookie 从未绑定 IP 返回 401、新 IP 登录到正确选手、恢复原座位及重复同步。最新脚本提交 5 得到真实 AC，重复调用复用回执。过期预览返回 409，离线目标显示原因，排队后目标变化在远端写入前失败，重新核对后执行成功。

真实浏览器完成管理员登录、核对、确认同步和逐人结果，截图为 artifacts/m3-domjudge-browser.png。证据位于 local/m3-domjudge/mapping-verification.json、failure-verification.json。已创建独立 jury/api_writer 服务账号（9.0.2 关闭公开排名时，读取全场 teams 需要 jury 权限），材料为 local/domjudge/wccs-api.json。当前管理端新功能仍在本地测试实例，线上 WCCS 保持 schema 13。

用户已确认暂停同时影响 DOMjudge 计时、提交及桌面。9.0.2 原生具有 RemovedInterval 计分能力，运行中暂停 API 与计时展示仍需接入和验证。比赛安排及 M5 尚未完成。

## M4 HTTPS 分发开发与首轮回归（2026-11-17）

新增资料上传、SHA-256 分块校验、重复上传续传、目标预览、发布时间、逐台限速、取消及补发页面。每块 256 KiB，单文件上限 512 MiB，内容存入 PostgreSQL 独立分块表，随数据库统一备份。服务器限制 16 个并发下载，并按设备限制分块速率。

Windows 服务新增独立下载线程，重用身份认证与目录重解析点防护。缓存位于受保护身份目录，校验完成后等待服务器发布许可。文件发布到比赛工作目录 WCCS-resources 下的独立版本目录，已有不同内容文件报告冲突。--distribution-test 仅写入身份文件旁的独立模拟目录。

首轮实际检查如下。

- scripts/check-m2.ps1 全部通过，含 Rust 构建、Clippy、Windows 原生文件回归、18 项数据库和 37 项前端测试及构建。
- Python/PowerShell 27 项通过，仓库文本检查与 git diff --check 通过。
- local/m4-distribution/verification.json 记录真实客户端模拟入口对本地服务器的端到端结果，包含上传续传、服务重启保留任务、断网保留缓存、损坏分块修复、发布前文件不可见、取消后明确补发和重复执行。1,664,011 字节文件内容一致。
- M4 管理页面真实浏览器完成上传、预览、提交及 7/7 分块发布核对，截图为 artifacts/m4-distribution-browser.png。Linux 最新分发原生及 18 项数据库检查通过。指定实机普通账户权限和大量文件容量继续待验。

M3、M4、M5 仍处于连续开发中，尚未进入全场部署。

## M3 比赛安排与暂停联动（2026-11-17）

新增 schema 16、持久比赛联动任务和“比赛安排”页面，复用 waiting 预览、权限、命令持久化和逐机回执。开始、暂停、恢复、结束支持立即执行或未来七天内安排。定时开始先关闭 DOMjudge 自动开赛和提交，到时核对冻结的设备身份、连接、会话及授权，桌面解除成功后开启比赛。该版本取消安排或服务重启后保持等待开赛。schema 21 及配套 DOMjudge 扩展支持明确取消并恢复原开关，见 [ADR 0016](../decisions/0016-contest-control-and-recovery.md)。

暂停和结束先关闭评测提交，再让所选桌面进入等待。恢复先核对所有所选桌面的解除回执，再恢复 DOMjudge。部分失败保留两端实际结果，DOMjudge 已暂停时不会因桌面失败自动恢复。断线重连后的连接编号变化使旧安排失效。

DOMjudge 9.0.2 扩展已安装到实际服务器，源码位于 integrations/domjudge。使用原生 RemovedInterval 记录暂停并顺延结束等未来里程碑。固定源码摘要修补 CLICS duration 和封榜时长，使暂停区间从比赛时长中扣除。暂停期间原生 end_time 暂用 2100 年内部占位值，WCCS 状态接口提供冻结的剩余时间，其他依赖原生 end_time 的外部系统接入时需采用该状态接口。恢复后使用实际结束时间。原始文件备份为 /var/backups/wccs/wccs-control-20261116T173623Z。

实际验证如下。

- 完整 Windows Rust 构建、Clippy、常规测试，19 项 PostgreSQL 集成、37 项前端及生产构建通过，日志为 artifacts/m3-control-check.txt。
- 真实 DOMjudge 暂停时计时冻结、提交拒绝、PHP-FPM 重载后保持暂停、过期版本返回 409、恢复后重放旧请求保持当前状态。CLICS duration 保持 5:00:00.000，原有提交的比赛时间保持一致。
- 独立本地 WCCS 与合成 WebSocket 客户端通过暂停/恢复、取消安排、重连拒绝旧安排、桌面失败保持评测暂停及明确恢复。证据为 local/m3-domjudge/control-verification.json。
- 私有合成比赛 wccs-control-test 验证定时开始提前关闭自动开赛、原 1 小时时长保留、桌面先解除及结束联动，现已结束。证据为 local/m3-domjudge/schedule-verification.json。
- 真实浏览器完成预览、确认暂停、双端成功回执和恢复，截图为 artifacts/m3-contest-control-browser.png。DOMjudge 自身页面读取仍超时，网页暂停时钟的可视验证待补，API 和 WCCS 页面已验证。

生产 WCCS 仍为 schema 13。最新功能运行在独立临时实例，M5 继续实施，指定实机的比赛控制、策略、文件权限和复原验证仍待完成。


## M5 恢复闭环与服务器交付（2026-11-17）

新增 schema 17/18、比赛登录策略选项、硬件变化告警与事件处置、赛后恢复页面。复用受控登录、maintenance 预览、权限、结束会话确认、持久回执和恢复链。原生策略先写恢复记录再修改可移动存储访问和软件关机权限，断网、教学恢复与封装共同恢复。恢复记录校验机器和权限条目完整性，未知冲突保留失败状态。

赛后任务按检查、归档或明确跳过、恢复教学、清理、重启顺序执行。检查冻结卷身份和完整文件清单。客户端拒绝重解析点、系统卷整卷目标、客户端自身目录与变化范围，持有文件句柄防止检查后替换。服务端逐块及整体 SHA-256 校验，通过后生成标准 ZIP。重启请求受理与重新上线核对分别展示。清理中断可能保留部分已删除事实，需重新检查，归档可下载恢复。能力限制与理由见 ADR 0016。

实际检查与证据如下。

- `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 通过，日志 artifacts/m5-final-check.txt。95 项常规 Rust、20 项 PostgreSQL 集成、37 项前端通过，包含构建、格式、Clippy、Provider 编译和前端生产构建。容量用例本轮过滤，原生 WSS 专项未在此命令运行。
- `pwsh -NoProfile -File scripts/check-rust.ps1` 与 `npm --prefix web run check` 在恢复日志完整性和重启状态文案修正后再次通过，日志 artifacts/m3-m5/rust-check.txt、web-check.txt。
- `python local/linux-deployment/check-field-linux.py` 返回 0，Linux Rust 和 20 项数据库通过，日志 artifacts/offline-client/check-linux.txt。
- `python -m unittest discover -s scripts/tests -v` 27 项通过，日志 artifacts/m5-python.txt。交付包增加现场说明和脚本清单后再次运行 27 项通过，日志 artifacts/m3-m5/python-check.txt。
- local/m5-recovery/verification.json 六组断言全部通过，实际客户端模拟入口验证 ZIP 内容、变化时拒绝删除、重复操作、明确跳过及重启模拟。原生文件句柄回归验证写入/重命名锁和清理中断后的剩余文件，诊断回归验证分发回执脱敏。
- 浏览器在独立测试实例完成检查、归档、清理及事件 178 已解决，截图 artifacts/m5-recovery-browser.png 及 artifacts/m3-m5/recovery-event-verified.jpg。开发机系统账户、服务、证书、策略及重启未执行本轮原生动作。
- M3 pause/resume、取消、重连拒绝及桌面失败保留远端暂停再次通过，artifacts/m3-m5/control-check.txt。重复使用已结束的定时测试比赛时新版本返回 409，符合阶段检查；此前定时开始/结束成功证据保留在 local/m3-domjudge/schedule-verification.json。

生产 WCCS 已从 7809210-image-1116.4 更新到 `/opt/wccs/releases/7809210-m3m5-1117.2`，数据库 schema 18。备份为 `/var/backups/wccs/m3m5-1117.2`。停止服务后的旧库备份恢复到独立 PostgreSQL 库通过，原有工具配置、准备配置、机房、座位、比赛、名单与分配摘要保持一致。只新增 DOMjudge 连接配置和 server.json 的对应字段，API 材料仅 wccs 可读。首次更新发现公共 CA 文件为 root-only，自动回退成功；调整公共证书读取权限后重试成功。管理 HTTPS、静态页面、新端点和六个系统服务核验通过。工具配置仍为 4，准备配置仍为 9，设备 3 台、机房 2 个。

DOMjudge 最新数据库及配置/扩展备份为 `/var/backups/wccs/domjudge-20261116T184506Z`，独立 MariaDB 恢复通过，提交 5、判题 5、测试点 15、控制状态 2、请求 21。备份包含控制器、时钟脚本、原生 Contest 修改、模板、autoload、认证设置和 CA。证据 artifacts/m3-m5/domjudge-backup.json。

M3～M5 本轮软件与材料已交付，软件关机会话恢复边界及最终现场验收继续待补。仅 503-41 执行更新后的普通账户真实编译/提交、双端暂停、USB/关机权限、断网恢复、目录清理、重启与 C 盘复原矩阵。DOMjudge 页面在内置浏览器的新标签仍超时，可视时钟验收待补。服务器整机重启、正式比赛配置、判题容量和长期稳定性未作为本轮已验事实。第一步见 m3-m5-field-guide.md。

## M3～M5 合并材料 v6（2026-11-17）

当前交付为 `artifacts/m3-m5/WCCS-503-41-M3-M5-v6.zip`，客户端版本 `0.3.0-m3m5.20261117.1`，109954735 字节，956 个清单文件。SHA-256 为 `7060481d00f66a50e07b3baecbfd2737dec6ac8f7bac015af23088319fe2c642`。包含既有镜像入口、完整运行时、公开 CA 和配置，新增 M3-M5-FIELD-GUIDE.md 与可导入的 contest-scripts-v1.json。

独立临时目录解包、逐文件摘要、所有记录源码摘要、脚本内容、封装载荷检查、逐机私密材料缺席及错误机器目标拒绝均通过，证据 artifacts/m3-m5/package-verification.json。先在 503-41 更新、诊断并完成 M3～M5 实机矩阵，再封装和保存代表机测试镜像，具体见 [单机说明](m3-m5-field-guide.md)。服务端当前 schema 18，完整记录见 [M3～M5 开发记录](m3-m5-implementation.md)。

## 软件关机权限恢复边界

Windows 用户权限分配在下一次登录生效。恢复 LSA 授权后，旧比赛会话令牌仍需重新登录才能获得关机权限。当前离线恢复保留程序与文件，教学管理员账户可正常使用。旧比赛会话原地恢复全部电源能力尚未完成，M5 该项继续待补，现场先保持软件关机选项关闭。实施完整恢复需补充受控本机电源入口或经确认的重新登录流程，当前包不把它计入已通过实机项。[Microsoft 用户权限说明](https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/shut-down-the-system)

## 客户端后续收尾（2026-11-17）

上节关机恢复已补充本机受控电源入口，同时扩展清理容量及镜像开机版本恢复。新软件状态与验证见 [稳定性记录](client-deployment-stability.md)，历史材料 v6 保留原实现说明。
