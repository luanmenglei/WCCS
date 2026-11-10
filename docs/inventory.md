# Windows 只读环境采集

`wccs-inventory` 是 M1 的首个可运行切片。采集器使用 Rust 和原生 Windows API，导出版本化 JSON 和中文脱敏摘要。当前支持目标为 Windows 10 x64 及更新系统，开发机运行证据见 [M1 任务记录](tasks/m1-inventory.md)。机房 Windows 10 镜像仍需现场验收。

## 构建与运行

在仓库根目录执行以下命令。构建需要 rustup、MSVC Build Tools 和 Windows SDK。运行发布后的 exe 可使用现有 Windows 账户，目标机需要可用的 VC++ x64 运行库。当前 exe 的导入表包含 `VCRUNTIME140.dll` 及 Windows 系统 DLL，干净机房镜像中的实际启动行为待验收。

```powershell
pwsh -NoProfile -File scripts/check-rust.ps1
& ./target/x86_64-pc-windows-msvc/release/wccs-inventory.exe --output-dir local/environment --context development
```

检查脚本固定 Rust 1.99.1、MSVC 工具集 14.44.35207 和 SDK 10.0.19041.0，验证所选环境，并运行 C / Rust 最小程序、发布构建、rustfmt、Clippy 及测试。`rust-toolchain.toml` 声明工具链和 MSVC 目标，`Cargo.lock` 固定全部 Rust 依赖。首次构建需下载对应工具链与依赖，准备好缓存后可使用 Cargo 离线模式。脚本在当前 PowerShell 进程设置编译环境，建议按上述独立进程方式运行。

日常开发也可逐项执行以下命令。逐项命令使用当前终端的 MSVC / SDK 环境，完整验收使用上面的固定版本脚本。

```powershell
cargo build --locked -p wccs-inventory --target x86_64-pc-windows-msvc --release
cargo fmt --all -- --check
cargo clippy --locked -p wccs-inventory --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo test --locked -p wccs-inventory --target x86_64-pc-windows-msvc
cargo run --locked -p wccs-inventory --target x86_64-pc-windows-msvc -- --output-dir local/environment
```

## 运行选项与产物

| 选项 | 含义 |
| --- | --- |
| `--help` | 查看帮助并退出 |
| `--output-dir DIR` | 报告父目录，默认当前工作目录下的 `local/environment` |
| `--context development` | 默认值，明确标记开发环境证据 |
| `--context lab` | 运行者将本次采集标记为机房证据，具体镜像和测试条件另行记录 |
| `--list-tools` | 单独使用，输出供管理端配置的候选工具 JSON 清单 |
| `--tools-config FILE` | 读取版本化工具配置，检查其中的完整文件绝对路径，支持中文和空格路径 |

每次运行生成独立的 `report-<时间>-<进程号>/`，包含 `report.json` 和 `summary.txt`。终端显示同一份脱敏摘要及报告位置。JSON 包含真实主机名、账户、SID、卷标识、网络地址和工具路径，应保存在忽略目录。仓库中的 `local/`、`artifacts/` 和 Cargo `target/` 已忽略。

导出先创建独占 `.partial` 目录，写入并刷盘两个文件后重命名发布。重复或并发运行各自产生新目录。中断或写入失败留下的 `.partial` 表示未完成导出，重试会新建报告。采集期间系统状态可能变化，各节表示依次读取到的状态。

退出码 0 表示报告已生成或清单已输出，局部字段仍可能缺失或失败。退出码 1 表示配置读取、配置校验、采集、导出或终端输出失败，退出码 2 表示参数错误。使用方应同时检查字段状态，摘要列出待处理字段的路径和错误码。

## 报告结构

采集器当前为 0.1.2，顶层 `schema_version` 为 3，另有采集器版本、Unix 毫秒时间、`evidence_context` 和 `collection_mode=read_only`。Schema 3 的工具区记录配置版本和逐项路径检查结果。读取模型保留 Schema 1 / 2 的历史工具结果和搜索目录字段，测试保留第一版合成样例及第二版脚本字段的往返验证。

| 分区 | 字段范围与解释 |
| --- | --- |
| `system` | 主机名、内核主次版本与 build、EditionID、DisplayVersion、UBR、系统原生架构及采集进程架构 |
| `account` | 当前账户名、进程令牌 SID、提升状态、提升类型、管理员组状态、会话 ID |
| `volumes` | 卷 GUID 路径、挂载路径、驱动器类型、卷标、序列号、文件系统及 flags、总量、空闲量、调用者可用量和枚举状态 |
| `network` | 全部接口的索引、名称、类型、运行状态、MAC、MTU、IPv4 / IPv6 地址和前缀长度、DNS、网关，保留 IPv6 scope ID |
| `tool_configuration` | 已使用的 `config_version`，未提供工具配置时为 `not_configured` |
| `tools` | 每项的 `id`、`name`、`kind`、`path` 和 `check`。成功时 `check.value.file_version` 提供文件版本或不适用状态 |

可失败字段采用 `status=available` 加 `value`，或 `status=unavailable` 加 `reason` 和 `code`。当前原因包括 `permission_denied`、`missing`、`not_configured`、`not_a_file`、`unsupported` 和 `system_error`。历史报告中的 `not_found_in_search_scope` 按原义保留。Windows 错误码尽量保留原始 Win32 数值，HRESULT / NTSTATUS 无 Win32 映射时保留无符号原值。未知权限保留 unavailable。卷枚举失败时已采集条目继续保留，完成状态记录在 `volumes.enumeration`。

脚本文件的 `file_version` 使用 `status=unavailable`、`reason=not_applicable`、`code=null`，摘要显示“不适用（脚本文件）”，该状态从失败字段计数中排除。

`account.administrators_group` 中 `enabled` 表示当前令牌启用了管理员组，`deny_only` 常见于 UAC 过滤令牌，`disabled` 表示组被禁用，`absent` 表示当前令牌没有该组。结合 `elevated` 和 `elevation_type` 解读当前进程的有效权限。登录方式、账户口令策略与登录界面能力由后续原型验证。

卷类型沿用 Win32 数值，3 为固定磁盘、2 为可移动介质、4 为网络盘、5 为光驱。卷 GUID 枚举覆盖本地卷，网络盘映射另待相应运维场景扩展。网卡运行状态 1 为 Up、2 为 Down，完整数值保留到 JSON。未配置的地址、DNS、网关和 MAC 可为空。摘要列出具有地址配置的网卡，JSON 保留所有接口。

## 工具清单与服务端路径配置

工具选择、路径和配置版本由服务端统一维护，客户端按收到的有效配置检查。当前 M1 提供本地 JSON 输入用于验证检查边界，M2 将交付管理页面、服务端存储、授权下发和结果回传。[ADR 0004](decisions/0004-server-managed-tool-paths.md)

[候选清单](../config/tool-catalog.json) 提供编译器、调试器、解释器、IDE、compilecpp 和 submit 等逻辑工具，初始路径全部为 null。管理员可保留所需条目、添加自定义 ID，填写实际客户端的完整文件路径及新的配置版本。

```powershell
& ./target/x86_64-pc-windows-msvc/release/wccs-inventory.exe --list-tools
New-Item -ItemType Directory -Path local -Force | Out-Null
Copy-Item -LiteralPath config/tool-catalog.json -Destination local/tools.json
# 编辑 local/tools.json，填写本次配置版本及工具文件路径
& ./target/x86_64-pc-windows-msvc/release/wccs-inventory.exe --tools-config local/tools.json
```

配置包含 `schema_version=1`、非空 `config_version` 和 `tools` 列表。每项包含唯一的 `id`、显示名 `name`、`kind` 和 `path`。种类可选 `executable` 或 `script`，客户端按这个明确类型决定版本资源的读取方式，文件名和扩展名由管理员配置。

`path=null` 或缺省表示尚未配置。已配置路径必须为完整的 Windows 文件绝对路径。客户端将该字符串直接交给文件查询 API。指向目录时报告 `not_a_file`，缺失文件时报告 `missing`。配置格式、版本、重复 ID 和相对路径错误在工具文件检查前终止本次运行。

未传入 `--tools-config` 时，报告记录工具配置未配置，工具结果列表为空。显式空列表则表示本次配置选用了零个工具。以前使用 `--tool-dir` 的调用应迁移到上述完整路径配置。

## 只读边界与实际能力

系统查询读取版本、注册表、进程令牌、卷元数据和本机网络配置。工具检查仅读取配置明确指定的文件元数据，`executable` 类型另读 PE 版本资源。程序的文件写入集中在报告导出目录。相同名称出现在其他目录时，当前报告仍按配置路径的实际结果记录。

PE 文件可能缺少版本资源，脚本资源使用 `not_applicable`。存在性与文件版本属于本切片的检查范围，实际编译、提交、解释器及 Python 模块依赖验证在 M3 的受控运行入口执行。脚本包版本、配置版本和依赖清单在发布流程中分别管理。

采集依次查询系统与文件，元数据可能在读取期间变化。配置中的网络共享和文件系统链接按 Windows 文件查询规则处理，其可达性和响应时间依赖实际存储环境。

摘要省略机器名、账户名、SID、卷 GUID / 序列号 / 卷标 / 路径、网卡名称 / 描述、IP / MAC 和工具路径。它保留操作系统版本、容量、接口数量及工具文件版本，适合项目内核对。合成格式样例位于 [测试 fixture](../crates/wccs-inventory/tests/fixtures/report.json)。

## 尚待现场完成

需要一台与机房一致的 Windows 10 x64 镜像，分别使用普通账户、未提升管理员和提升后的管理员运行采集；对照信息中心记录核对版本、D 盘 GUID、挂载点和权限。再在测试镜像验证登录、锁定、复原、客户端重装和 D 盘重解析点。

网络放行验证需要真实 VLAN / ACL 配置及指定服务端和 P2P 端口。Linux 验收需要目标发行版、架构和可运行环境。上述条件及下一切片保存在 [路线图](roadmap.md)。
