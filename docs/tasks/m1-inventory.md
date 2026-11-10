# M1 只读环境采集切片

日期为 2026-11-10。状态为切片完成，开发机验证通过。目标为可运行的 Windows 环境采集 CLI，提供版本化 JSON 和脱敏摘要。

## 实施范围

- 创建最小 Cargo workspace，固定 Rust 1.99.1 和 x86_64-pc-windows-msvc 目标。
- 使用 windows 0.61.3 原生 API 绑定、serde 1.0.228 和 serde_json 1.0.149。现有仓库只有基础检查脚本，以上依赖承担平台接口、类型序列化和 JSON 编码。
- 采集系统版本、架构、进程账户及权限、卷身份和挂载点、网络配置及竞赛工具文件。
- 文件探测覆盖 PATH、常见安装目录及显式附加目录，读取 PE 版本资源。采集调用均为查询，不运行被探测工具。
- 按字段保留权限不足、缺失值和其他 API 错误。真实报告保存到忽略目录 `local/environment/`。
- 使用合成数据验证失败路径，并在开发机运行真实 API 与完整 CLI。

## 实际验证

架构与约束修改先通过仓库检查及 14 项基础测试，提交为 `dd6ae84`。随后在 `feat/environment-inventory` 分支完成本切片。

| 命令或验证 | 结果 |
| --- | --- |
| `rustc --version --verbose`、`cargo --version`、`rustup show` | Rust / Cargo 1.99.1，目标 x86_64-pc-windows-msvc |
| `vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath` | 找到 VS 2022 Build Tools，MSVC 工具集 14.44.35207 |
| 读取 Windows Kits 安装目录 | 已安装 SDK 10.0.19041.0 和 10.0.26100.0 |
| `link.exe /?` | 链接器 14.44.35222.0 |
| `pwsh -NoProfile -File scripts/check-rust.ps1` | 通过，脚本显式选择 SDK 10.0.19041.0 和 MSVC 14.44.35207 |
| 脚本内 C 最小程序 | `cl /W4 /WX` 使用 `windows.h` 编译、链接 kernel32，实际运行成功 |
| 脚本内 Rust 最小程序 | `rustc --target x86_64-pc-windows-msvc -C linker=...` 编译链接并输出 `WCCS Rust MSVC SDK smoke OK` |
| `cargo build --locked -p wccs-inventory --target x86_64-pc-windows-msvc --release` | 通过，exe 为 406528 字节 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked -p wccs-inventory --target x86_64-pc-windows-msvc --all-targets -- -D warnings` | 通过 |
| `cargo test --locked -p wccs-inventory --target x86_64-pc-windows-msvc` | 17 项通过，包含 11 项库测试、3 项参数测试、3 项 CLI 集成测试 |
| `python scripts/check_repo.py`、`python scripts/check_repo.py --staged` | 通过，37 个仓库文件 |
| `python -m unittest discover -s scripts/tests -v` | 14 项通过 |
| `git diff --check`、`git diff --cached --check` | 通过 |
| `dumpbin /dependents target/x86_64-pc-windows-msvc/release/wccs-inventory.exe` | 依赖 Windows 系统 DLL 和 `VCRUNTIME140.dll` |

发布程序实际执行命令如下。

```powershell
& ./target/x86_64-pc-windows-msvc/release/wccs-inventory.exe --output-dir local/environment --context development
```

验收报告保存在 `local/environment/report-1794282543890817800-93500/`，包含完整 JSON 与脱敏摘要。最终最小程序证据位于 `artifacts/toolchain-smoke/d016055e61e341c39b6056bec4e5e28a/`。这两处与 Cargo 产物均已通过 `git check-ignore` 核对。发布 exe 的 SHA-256 为 `248A7648F80A69D4674E2A84952C786DBE4F888D328647E9F77C1CB75BA977EA`。

当前开发机实测内核为 10.0.26300.9711、x86_64，当前进程为未提升管理员令牌，采集到 5 个本地卷和 57 个网络接口。使用 `Get-CimInstance Win32_OperatingSystem` 比对系统版本，使用 `Get-CimInstance Win32_Volume` 比对全部卷 GUID，使用 `Get-NetIPAddress -AddressFamily IPv4` 比对 IPv4 集合，使用当前 `WindowsPrincipal.IsInRole(Administrator)` 比对有效管理员权限，全部一致。另从真实 JSON 提取主机名、账户名、SID、卷 GUID、IPv4，核对摘要均未包含这些值。

工具检测中部分文件缺少 PE 版本资源，部分可执行文件未出现在搜索范围，均在字段状态和摘要中明确报告。当前证据标记 `development`。

## 自审与修复

- 首次真机运行发现零长度 `TokenElevation` 探测返回 `ERROR_BAD_LENGTH`。改为固定大小结构直接查询，并补充真实当前进程回归测试，现已通过。
- 权限拒绝测试向令牌查询边界注入 `ERROR_ACCESS_DENIED`，保留可用用户名并将全部权限字段标为未知。工具目录拒绝访问、工具未发现及缺失版本资源分别验证。
- 为逐目录工具查询错误保留目录和错误码，报告已有命中结果时仍保留其他目录的失败证据。
- 对 `VerQueryValueW` 失败使用明确的资源缺失错误，避免读取未保证更新的 last-error 值。
- FFI 集中于 platform 模块，审查对齐缓冲区、长度、IPv4 字节序、IPv6 scope、SID 分配释放及 token / volume 枚举句柄所有权。各处 unsafe 带有前置条件注释，纯模型、工具搜索、摘要、导出和 CLI 禁止 unsafe。
- 网络与令牌可变长缓冲区最多重试 3 次，容量上限 16 MiB。挂载点缓冲区最多重试 3 次、上限 1 Mi UTF-16 单元，失败显式记录。
- 导出使用独占新目录、create_new 文件和完成后目录重命名。重复导出与目标已是文件的失败路径均通过测试，原有内容保留。失败遗留目录使用 `.partial` 标记。
- 实际 CLI 测试使用独立中文空格临时目录和合成工具文件，验证工具文件内容保持一致、JSON 可解析、摘要脱敏及参数错误时的目录状态。测试仅清理自身临时目录。
- 系统适配器均为原生查询，文件写入集中在导出模块。查询过程使用同步系统调用，文件系统响应时间由实际存储环境决定。

可变长网络查询允许配置在调用间变化，故采用有上限的扩容重试。管理员组的 deny-only 属性保留在报告中用于解释 UAC 令牌。[GetAdaptersAddresses](https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getadaptersaddresses)、[令牌 SID 属性](https://learn.microsoft.com/en-us/windows/win32/secauthz/sid-attributes-in-an-access-token)

## 后续条件

| 项目 | 所需条件及下一步 |
| --- | --- |
| Q-01 系统兼容 | 与现场一致的 Windows 10 x64 镜像，核对 VC++ x64 运行库并运行发布 exe |
| Q-02 账户与登录 | 普通账户、未提升管理员、提升管理员及测试登录策略，分别采集并开展会话原型 |
| Q-03 复原 | 复原产品、版本、覆盖范围及可回滚镜像，执行安装和重启验证 |
| Q-04 网络 | 真实 VLAN / ACL、服务地址、服务器及 P2P 指定端口，执行受控连通性验证 |
| Q-08 D 盘 | 现场卷报告及单独测试卷，核对卷身份、目录重解析点、占用和恢复条件 |
| Q-06 Linux | 目标发行版、CPU 架构和可运行的 Linux 环境，随后引入服务端工程并验收 |

下一可执行切片为 Rust Win32 用户会话等待界面。CLI 使用说明与字段约定见 [采集工具说明](../inventory.md)。
