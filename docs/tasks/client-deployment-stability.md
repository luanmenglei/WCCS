# 客户端部署前稳定性收尾

2026-11-25 更新：文中的 503-41 专用包、命令及原定现场步骤仅供追溯。本轮部署排除该机，具体流程和目标待定。

2026-11-17，用户要求批量部署后避免客户端大幅变更，先闭合开发侧缺口，现场验收暂缓。本轮从 `78eafb0` 创建 `fix/client-deployment-stability`。

## 本机电源恢复

普通比赛账户登录时已缺失的关机权限无法加入既有令牌，Windows 原生菜单的权限刷新仍依赖重新登录。本轮补充本机 `wccs-agent --power shutdown|restart`，桌面准备自动生成“恢复后关机”和“恢复后重启”快捷方式。保留原桌面、程序和文件的同时，恢复后可通过这两个入口执行电源操作。[Microsoft 令牌说明](https://learn.microsoft.com/en-us/windows/win32/secbp/changing-privileges-in-a-token)

独立本机命名管道拒绝远程连接，按实际连接令牌的 SID 和活动控制台会话鉴权。服务启动时入口默认关闭，教学及策略恢复成功后仅允许已记录的比赛账户使用。准备下一轮控制前关闭入口，限制与电源提交通过互斥状态串行。客户端弹出确认，SYSTEM 提交延迟 15 秒的本机请求，使用不强制关闭应用的 Windows API。此入口依赖本机 WCCS 服务运行，服务器离线时可用。服务停止或恢复失败时显示不可用，不回报成功。

权限判断有单元回归。实际 Windows 10 的管道权限、快捷方式、电源确认和未保存程序行为属于后续指定实机验证。

## 清理容量与协议

`recovery-v2` 使用任务内冻结的数量、归档容量和执行时限。页面默认 100000 个文件及目录、选定源文件 512 MiB、3600 秒，数量可在 1～100000、归档在 0～512 MiB、时限在 60～86400 秒内调整。清理总字节数不设 512 MiB 限制，所有文件仍逐块计算整体 SHA-256，选定归档文件额外保存分块摘要。跳过归档仍需填写原因并取得服务端回执。

容量上限约束内存、打开文件句柄和归档缓冲。归档下载沿用内存 ZIP，因此本轮保留源文件归档 512 MiB 上限。更大清理文件会增加扫描时间，应按页面调整时限。扫描期间保留文件及目录句柄，检查与删除之间的内容变化、重解析点、卷变化和客户端目录保护继续生效。目录枚举逐项计数，避免先把超大目录完整读入内存。卷根路径独立校验，允许 `D:\` 这类非系统卷目标，系统卷由客户端根据实际卷身份拒绝。

旧任务省略 `limits`，继续使用原 4096 项和总内容 512 MiB 语义。旧客户端在新任务预览中显示不支持，既有功能保持原协议。新客户端向旧服务端回退到旧能力清单。新增任务日志写入独立 `maintenance-v2` 目录，旧客户端回退时保留这些记录，新客户端再次启动可恢复和补报。

大清单通过摘要续租接口确认冻结命令，最长一秒刷新本轮授权，网络失败后停止。停止服务标志和单调时限每次检查，不依赖日历时间延长授权。

## 镜像与升级恢复

开机任务依据完整安装回执、机器身份和状态版本选择已有安装。升级后优先使用该版本的受保护支持包，普通重启沿用已安装二进制。升级过程保存完整支持包，未完成的二进制事务先走既有回退。旧升级仅保存二进制时，使用状态版本兼容的基线脚本启动已安装程序。

C 盘完整复原仍按镜像中的版本重新安装，随机生成本轮注册材料并集中审核。保持升级版本需要保存更新后的复原镜像。账户 SID、SAM/DPAPI、配置文件、复原软件和机器名的现场事实仍需代表机确认。

## 检查记录

本轮实际检查如下，日志保存在忽略目录 `artifacts/`。

- `$env:RUST_TEST_THREADS='1'; pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 通过 Windows 发布构建、格式、Clippy、100 项常规 Rust、21 项 PostgreSQL HTTP、37 项前端检查和生产构建，并构建 Credential Provider。日志为 `client-stability-m2-final.txt`。首次并行数据库检查遇到旧测试的 schema 清理死锁，串行重跑全部通过。
- `python -m unittest discover -s scripts/tests -v` 通过 27 项 Python/PowerShell 测试，含新安装、未完成安装、升级后重启保留版本、身份冲突和中断升级回退选择。日志为 `client-stability-python-final.txt`。
- `python local/linux-deployment/check-field-linux.py` 在既有隔离 Linux 测试目录执行 `python3 scripts/check-linux.py`，原生构建、格式、Clippy、常规测试与 21 项数据库测试通过。日志为 `offline-client/check-linux.txt`。
- `python -X utf8 local/m3-domjudge/verify-recovery-v2.py` 在本机测试服务与独立临时目录验证检查、选定内容 ZIP、变化后拒绝删除、精确清理和重复派发、模拟重启回执、明确跳过归档、扩展容量共 7 项。新任务产生 7 个 `.maintenance-v2` 日志，旧目录为空。日志为 `client-stability-recovery-final.txt`。
- 浏览器在本机 `#/recovery` 选择合成设备、设置 10000 项容量、预览并执行检查，观察到完成回执。进入下一步后数量、归档额度和时限保持原值并锁定。
- 容量回归在 Windows 临时目录扫描 4100 个目录及一个 512 MiB + 1 字节文件，验证新清理范围可用、旧容量拒绝、归档超额拒绝和检查期间写入受阻。100000 项为配置上限，200～300 台满容量吞吐继续待现场容量验证。
- Nginx 模板以临时证书、日志目录和非特权测试端口运行 `nginx -t` 通过，日志为 `client-stability-nginx.txt`。`python scripts/check_repo.py` 和 `git diff --check` 通过。

启用 recovery-v2 需要同时部署本轮后端、Web 和 Nginx 配置。回执上传的专用入口放宽到 128 MiB，其他接口沿用既有限制。开发检查使用隔离测试服务，后续实际部署结果见下方服务端更新记录。

原生电源、实际复原及全场容量保持待验状态。下一步按用户安排，在指定代表机验证新增快捷方式、升级后普通重启和 C 盘真实复原，再决定客户端冻结。


## 本地交付

源码提交为 `0e1ff85`，分支为 `fix/client-deployment-stability`。客户端版本为 `0.3.0-stable.20261117.1`，固定静态 CRT、MSVC/SDK 和 PowerShell 7.6.9，通过以下命令生成。

```powershell
pwsh -NoProfile -File scripts/build-client.ps1 -Version 0.3.0-stable.20261117.1 -PowerShellRuntime 'E:\project\WCCS\local\offline-client-tests\actual package 4\runtime' -Output artifacts/client-stability/wccs-client-0.3.0-stable.20261117.1.zip
python scripts/package-image.py --client artifacts/client-stability/wccs-client-0.3.0-stable.20261117.1.zip --ca local/linux-deployment/ca.crt --mother-target local/m3-m5/mother-target.json --output artifacts/client-stability/WCCS-503-41-stability-v1.zip
python local/m3-m5/verify-stability-package.py
```

- 通用客户端包 `artifacts/client-stability/wccs-client-0.3.0-stable.20261117.1.zip`，109888157 字节，SHA-256 为 `09201587362e8a9881e29f3092d065f1e2a283a4211401aedab042d568a207c9`。
- 503-41 配套包 `artifacts/client-stability/WCCS-503-41-stability-v1.zip`，109975684 字节，SHA-256 为 `3f3562bb14b4f61da6af615dc2ecf1455c0be80bf98100d21396a7d576d5840b`。沿用既有明确母机目标和公开 CA，包含 956 个清单文件。
- 实际解包验证通过所有文件大小与摘要、构建源码摘要、嵌入脚本逐字一致、密封载荷 Check 和错误机器身份拒绝，设备私密材料检查通过。结果见 `artifacts/client-stability/package-verification.json`。

客户端材料保存在本地，现场 Windows 机器按用户安排更新和验收。


## 服务端更新（2026-11-17）

用户接续要求立即更新。已在既有 `10.16.37.123` 部署后端、Web 和 Nginx，当前发布目录为 `/opt/wccs/releases/7b6418e-stability-1117.1`，前一版本为 `/opt/wccs/releases/7809210-m3m5-1117.2`。部署前核对没有活动等待、维护或比赛安排任务，按既有发布方式短暂停止 WCCS 后切换目录，启动成功后重载 Nginx。

- 核对远端 116 个构建源文件摘要与本地一致，重新执行 Linux `cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p wccs-server`。后端 SHA-256 为 `e1483e63fc1c744578adef2e25f2da8f106439eed0eaa539b7f00f118d9c1128`。
- 数据库与配置备份位于 `/var/backups/wccs/stability-1117.1`，包含 PostgreSQL dump、原 `/etc/wccs`、Nginx 配置、旧发布路径和校验回执。备份在独立数据库完整恢复后核对 schema 和业务表摘要，通过后清理该验证库。
- schema 保持 18，`/etc/wccs` 文件摘要及工具、准备配置、机房、座位、比赛、选手和分配表摘要核对一致。工具版本 4、准备配置版本 9、3 台设备和 2 个机房保留。
- `nginx -t`、服务健康、真实 CA 验证的 HTTPS、管理员登录、状态与恢复 API 通过。线上 HTML、JS 和 CSS 与本地构建逐字一致。
- 新摘要续租接口处理 recovery-v2 参数并正确拒绝缺少设备凭据的请求。超过 256 KiB 的合成回执通过 Nginx 进入设备鉴权，确认专用 128 MiB 上传限制已生效。检查请求仅用于验证接口及鉴权。
- WCCS、Nginx 和 PostgreSQL 均为 active。Windows 新版客户端仍通过前述安装包交付，原生现场验收继续按用户安排。

实际命令为 `python local/client-stability-deploy/deploy.py`（远端 root 执行）及本地 `python local/client-stability-deploy/verify.py`。脱敏部署和 HTTPS 检查回执位于 `artifacts/client-stability/server-deployment.json`、`server-verification.json`。本次同 schema 切换的失败处理恢复旧发布目录和原 Nginx 配置，旧发布及备份保留。
