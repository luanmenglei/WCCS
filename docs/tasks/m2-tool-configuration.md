# M2 工具配置管理闭环

日期为 2026-11-10。Windows 开发机闭环已实现，Linux 实际运行验收待具备环境后补齐。

## 起点与交付

核对基线提交 `8da0e7318dcc3ca729edb02c59ba875a9c413f89` 和 `951fc2433441b903b01c5f09d9f54b6de833fc81`。开始时工作区干净，由 `feat/session-waiting` 建立 `feat/tool-configuration`。

交付包括 Rust 服务和 PostgreSQL 迁移、React 管理界面、共享协议、Windows 检查入口，以及构建和回归命令。产品职责为服务端维护完整路径、客户端按快照逐项检查，未配置时返回 `not_configured`。实现选择与后续边界见 [ADR 0005](../decisions/0005-m2-tool-configuration-slice.md)。

## 环境

| 项目 | 本轮实际情况 |
| --- | --- |
| Windows | 当前开发机，MSVC 14.44.35207、SDK 10.0.19041.0，真实文件检查及 PE 版本读取 |
| Rust | 1.99.1，Windows MSVC 构建运行，Linux GNU 目标交叉类型检查 |
| PostgreSQL | 18.4，已有 `D:/PostgreSQL/18/bin` 二进制，独立 `local/m2/postgres` 测试集群，监听 `127.0.0.1:55432`，SCRAM 密码认证 |
| 前端 | Node.js 26.13.0、npm 11.20.0，生产静态资源由 Axum 提供 |
| 浏览器 | 实际 Chromium 内嵌浏览器，通过 Browser Use 操作页面 |
| Linux | WSL 未安装，无 Docker / Podman 命令和服务，无 SSH 配置或用户提供的远端目标 |

WSL 探测使用 `wsl --list --verbose`，容器及 SSH 使用命令发现、服务与配置检查。当前任务没有可用的远端连接目标。

## 启动开发闭环

以下命令在仓库根目录执行。现有开发实例的配置和凭据位于忽略目录 `local/m2/`。

### 构建与检查

```powershell
pwsh -NoProfile -File scripts/check-rust.ps1
npm --prefix web ci
npm --prefix web run check
```

前端固定 Node.js 26.13.0 与 npm 11.20.0，`.npmrc` 开启 engine-strict。Rust 脚本选择固定 MSVC 与 SDK，构建全部五个 workspace 包。前端构建输出为 `web/dist/`。

界面开发可用 `npm --prefix web run dev`，Vite 绑定回环地址并代理 `/api` 到本机 8787，将开发代理 Origin 设为该服务地址。生产验收使用 Axum 提供的 `web/dist`。

### 数据库和服务

已有测试集群停止后，可运行下列命令启动。该目录来自本轮独立 initdb。

```powershell
& 'D:/PostgreSQL/18/bin/pg_ctl.exe' -D E:/project/WCCS/local/m2/postgres -l E:/project/WCCS/local/m2/postgres.log -o '-p 55432 -h 127.0.0.1' start
& './target/x86_64-pc-windows-msvc/release/wccs-server.exe' --config local/m2/server.json
```

浏览器打开 `http://127.0.0.1:8787/`。管理员登录材料在 `local/m2/admin-access.json`，数据库连接在 `local/m2/server.json`。这些文件含开发凭据，存放在受保护的忽略目录。运行中的服务占用二进制时，重建前先退出对应进程。

新环境先创建独立 PostgreSQL 数据库及账号，再生成服务配置。

```powershell
& './target/x86_64-pc-windows-msvc/release/wccs-server.exe' --init local/m2-new/server.json
```

在生成的配置中填写 `database_url`，按实际部署修改 `bind`、`origin` 和 `static_dir`。默认连接本机 55432 的 `wccs` 数据库。初始化命令拒绝覆盖现有配置或登录材料。当前服务启动事务性创建版本 1 并升级到版本 3，已有版本 1 / 2 数据自动迁移，遇到未知 schema_version 拒绝启动。机房切片升级情况见 [座位表记录](m2-room-seating.md)。

### Windows 检查入口

本轮合成设备的身份文件为 `local/m2/device.json`。

```powershell
& './target/x86_64-pc-windows-msvc/release/wccs-agent.exe' --identity local/m2/device.json
```

加 `--once` 完成一轮同步后退出。持续模式前台运行，Ctrl+C 退出。后台运行时保留进程编号，结束该实例后可用相同身份文件重新启动。

新设备从页面“新增设备”填写稳定 ID 和名称，下载注册材料到该设备的受保护目录，再将路径传给 `--identity`。首次运行会在旁边生成 `.credential`，注册前持久化随机设备令牌。`.lock` 使用 Windows 独占句柄控制并发，文件可以长期存在。`.pending.json` 保存未确认结果，`.completed` 保存最近确认任务编号，`.pending.tmp` 用于发布前暂存。

注册材料有效 24 小时，一台设备注册后使用独立凭据心跳。重新使用注册材料生成不同凭据会返回冲突。复制已注册凭据将复制设备身份，部署镜像应在克隆后逐机配置注册材料。

### 浏览器操作路径

1. 管理员登录，进入“工具配置”。
2. 从候选清单添加工具，或通过“自定义工具”填写 ID、显示名称和文件种类。
3. 编辑工具完整 Windows 文件路径，留空表示未配置。
4. 选择已注册目标设备，点击“发布并检查”。页面显示当前配置版本与每台设备期望版本。
5. Windows 入口下一次心跳获取配置并检查，页面显示实际版本、逐项状态、文件版本和失败码。
6. 点击设备的“重新检查”生成同版本的新任务。此前结果显示过期，收到新任务结果后更新。
7. 多管理员发生版本变化时保留当前草稿，提示重新加载最新配置。服务失联时保留最后快照并禁用写操作。

当前只有一份全局工具配置，发布时替换整个目标集合。一次发布最多 64 个工具和 500 个目标，目标必须已注册。浏览器轮询刷新，设备检查与回传需要独立 Windows 进程运行。

## Linux 启动入口与缺失验收

准备 Linux x86_64、Rust 1.99.1、构建工具及 PostgreSQL 18.4，按机器安装对应 Node.js 26.13.0/npm 11.20.0，或复制已构建 `web/dist`。

```bash
python3 scripts/check-linux.py
./target/x86_64-unknown-linux-gnu/release/wccs-server --init local/m2/server.json
./target/x86_64-unknown-linux-gnu/release/wccs-server --config local/m2/server.json
```

第一次启动前编辑生成的 `database_url` 与静态资源目录。设置 `WCCS_TEST_DATABASE_URL` 后，`check-linux.py` 额外执行 PostgreSQL HTTP 集成测试。变量值只指向独立测试数据库。

远程浏览器与客户端通过有效受信任证书的同源 HTTPS 代理访问服务。设置 `origin` 为实际 HTTPS origin，代理转发到 Axum。当前数据库驱动使用 NoTls，数据库部署在同机回环或 Unix socket。Linux 非 root 运行、文件权限、HTTPS 代理、证书信任与 systemd 生命周期需要实际 Linux 环境验证。

本轮已执行 Linux 目标 `cargo check`，原生构建和实际 Linux 服务运行尚未执行。

## 验证记录

```powershell
pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json
cargo check --locked -p wccs-server -p wccs-protocol --target x86_64-unknown-linux-gnu
npm --prefix web audit
python scripts/check_repo.py
python -m unittest discover -s scripts/tests -v
git diff --check
git diff --cached --check
```

`check-m2.ps1` 从配置读取数据库连接到临时进程环境，运行固定 Windows 工具链检查、独立 PostgreSQL HTTP 集成测试、npm ci 和前端检查，结束后恢复原环境变量。集成测试创建随机独立 schema，成功后删除自身 schema。失败时保留该测试 schema 供诊断。

| 证据 | 实际范围及结果 |
| --- | --- |
| Rust Windows 检查 | release 构建、rustfmt、Clippy `-D warnings`，36 项测试通过 |
| PostgreSQL HTTP 集成 | 1 项综合生命周期测试通过，真实数据库与 HTTP，设备结果使用合成协议数据 |
| 前端检查 | 类型检查、ESLint、Prettier、5 项协议和组件测试、生产构建通过 |
| 依赖审计 | `npm audit` 0 项漏洞 |
| Linux GNU 目标 | Windows 宿主执行交叉类型检查通过 |
| 真实浏览器与 Windows | 登录、候选选择、自定义脚本、路径编辑、目标选择、发布与实际文件检查回传 |
| 部分失败 | v1 包含真实 PowerShell 文件存在及版本、submit 未配置、自定义路径缺失和系统码 2 |
| 断线及过期状态 | 停止实际客户端后显示离线；发布 v2 后显示期望 v2 / 实际 v1 / 结果已过期 |
| 并发编辑 | 第二标签页保留未发布草稿，显示基准 v1 与当前 v2，阻止旧草稿发布，重新加载可恢复 |
| 服务中断与恢复 | 关闭真实服务进程后页面显示状态过期并禁用写操作，重启后旧会话失效；重新登录后配置和结果仍可见 |
| 真实客户端重连 | 同一身份文件再次启动，自动恢复心跳并执行离线期间发布的 v2，中文空格路径脚本返回文件存在 |
| 浏览器重新检查 | 同版本新任务下发后旧结果暂时过期，实际客户端回传后恢复期望/实际 v2 |
| 客户端重试回归 | 实际 Windows 子进程对合成 HTTP 服务检查临时文件，模拟结果响应丢失，再次运行重发完全相同的持久化结果 |

HTTP 集成还覆盖无会话 401、错误密码、来源/CSRF 403、错误设备令牌、重复注册、注册身份接管拒绝、跨设备凭据重复绑定拒绝、非法路径、重复请求编号内容冲突、并发发布 200/409、结果设备不匹配、配置字段不匹配、重复结果冲突、旧结果晚到、心跳离线恢复、服务重建后配置/结果/去重持久化、旧会话失效、任务过期 410、退出后会话失效。

日志位于忽略目录 `artifacts/m2-full-check.txt`、`artifacts/m2-linux-cross-check.txt` 和 `local/m2/*.log`。浏览器截图位于 `artifacts/m2-browser.png`。设备 ID `synthetic-win-01` 与名称“合成机房 A · 01”为合成数据。开发机文件版本及完整运行报告保存在忽略目录。

## 自审与修复

- 服务端需要独立校验 Windows 完整路径。共享验证避免 Linux 的 Path::is_absolute 错误拒绝盘符路径，采集器继续执行本机路径校验。
- 结果模型沿用采集器扁平 JSON。浏览器首轮暴露嵌套字段误解后修正，Rust 与前端测试共同读取同一结果样例。
- pending 重传成功后写入 completed，避免使用旧心跳快照再次执行同一任务。
- 客户端锁采用 Windows 句柄独占，异常退出自动释放。
- 每台新增设备独立生成注册令牌。请求失败保留令牌供幂等重试，成功后轮换页面中的待用令牌。
- 校验请求幂等、配置版本和任务编号，保留历史报告，阻止迟到结果覆盖较新证据。已完成任务超过执行期限后仍保留完成证据，期限只阻止尚未完成的任务执行。
- 数据库唯一索引阻止跨设备重复绑定相同凭据；Argon2 校验并发限制为 2，任务取消时运行中的校验继续持有配额。
- 依赖审计暴露 Zod 与 minimatch 的已知问题，升级并固定兼容 override，重跑前端检查和 npm ci。

## 本地提交与当前实例

- `416bc1f` 实现 Rust 服务、协议、Windows 检查入口、持久化与回归。
- `d8631ac` 实现 React 管理界面、锁定依赖和完整检查脚本。
- 本任务记录、路线图和受影响架构文档随交接提交保存。

交付时保留本地 PostgreSQL、8787 服务及合成 Windows 客户端运行，服务和客户端进程编号分别保存在 `local/m2/server.pid`、`local/m2/agent.pid`。停止前核对进程路径，再停止该实例。Vite 开发代理已验证登录和状态读取，验证后关闭 5173 开发服务。

## 下一项

先在可用 Linux 测试机执行原生构建与 PostgreSQL 集成，再完成 HTTPS 代理、浏览器至 Linux 服务和 Windows 客户端的跨机闭环。随后推进 M2 的设备凭据生命周期、WSS 与 Windows 服务、鉴权 IPC 和等待界面批量控制。机房 Windows 10 镜像、普通账户、复原后的身份恢复和现场网络继续作为独立验收条件。

仓库检查通过 82 个文件，Python 仓库工具 14 项回归通过，工作区及暂存区 diff 检查通过。浏览器窄屏覆盖待补，本轮视口覆盖实际桌面窗口。


## 机房座位表切片补充

后续 M2 已增加矩形机房、稳定座位 ID、IP 绑定及认证来源匹配，保留本记录的全局配置与完整目标集合语义。座位检查使用独立的目标预览与提交入口，只重查配置范围内的唯一在线设备，其余目标明确跳过，详见 [座位表记录](m2-room-seating.md) 和 [ADR 0006](../decisions/0006-m2-room-seating.md)。

## 当前注册与客户端版本

客户端 0.2.0 已加入持久注册审核和凭据轮换。原材料格式与 pending / completed 文件兼容，默认关闭注册窗口，旧注册设备升级后继续启用。新申请和重装恢复的操作方法见 [设备生命周期记录](m2-device-lifecycle.md)。本文件先前的注册行为描述保留首轮历史证据，当前状态语义以 ADR 0007 为准。
