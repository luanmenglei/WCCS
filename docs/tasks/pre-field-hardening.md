# 首次实机测试前加固

2026-11-13 后续已完成首次实机离线包、显式目标安装/升级回退、自检和一次性 SYSTEM 教学恢复入口。当前执行版本、摘要与软件回归见 [离线客户端交付](offline-client.md)，首次现场使用 [逐机操作表](offline-client-field-guide.md)。原 VM 名称约束已移除，虚拟化环境保持结束状态。

2026-11-13 用户决定结束虚拟化验证，清理现有环境并继续推进开发，尽量在首次实机测试前发现软件问题。沿用 `feat/linux-deployment`，本轮继续保留已有工作区改动。

## 虚拟化环境清理

核对进程路径和命令行后停止 QEMU `WCCS-Login-Test` 及回环实验 HTTP 服务。删除前核对三个目录的绝对路径、父目录及重解析点，归档小型脚本和结果并校验条目数量与长度，再清理 `local/login-vm`、`local/native-linux`、`local/machine-tools-vm`。

共清理 4,038 个文件，逻辑文件大小 54,124,081,424 字节，约 50.4 GiB，包括基础磁盘、两个覆盖层、Windows ISO、QEMU 及来宾安装材料。591 份小型证据归档到忽略目录 `local/environment/retired-vm-evidence-20261113.zip`。归档 SHA-256 为 `8DB491E64144E4ECDF0053E256F1D96EA558F4D599F57B21DBE69F4DD129467F`。原有 `artifacts/` 屏幕和测试证据继续可查，历史文档中的三个 local 目录路径对应归档内同名条目。

通过原管理鉴权接口撤销 Linux 测试服务器上的 `login-native-01`，设备历史保留、凭据失效、IP 占用释放。收尾检查确认 QEMU/实验服务进程结束，8870/8871 无监听。清理命令和记录位于 `local/environment/retire-vm.ps1`、`vm-cleanup-20261113.json` 和 `retired-vm-device-receipt.json`。

## 多客户端控制回归及修复

复用现有 PostgreSQL 隔离 schema、真实 HTTP 管理接口和 WebSocket 协议测试设施，新增 `concurrent_control_preview_commit_and_reconnect`。每个模拟设备使用独立凭据，每秒交换控制状态。测试批量预览、提交、重复 commit、逐机完成、审计事件数量、管理列表持续读取和全部设备以新宿主实例重连，核对旧授权取消。

首轮 300 客户端的命令接收 p95 为 2,576 ms，超过验证计划的 2 秒目标。定位并修复以下开销。

- 设备权限范围改为一次查询全部选中设备，并逐项执行既有范围校验。
- 批量命令在同一事务内一次分配序列、取消旧命令并插入新命令，逐机事件继续由既有触发器记录。全部新命令采用同一签发时间。
- 单设备控制心跳仅处理该设备的过期和授权失效命令。管理列表继续处理全场设备，包括离线设备。

Windows 开发机与本机 PostgreSQL 18.4、回环 WS、debug 测试构建的初步采样如下。统计窗口从首次批量提交开始，接收延迟包含提交处理时间。现场目标为 300 客户端 p95 ≤ 2 秒，500 客户端用于容量余量观察。

| 场景 | 命令接收 p95 | 心跳往返 p95 | 实际结果 |
| --- | --- | --- | --- |
| 原实现，300 客户端，15 秒 | 2,576 ms | 1,388 ms | 延迟目标失败 |
| 批量提交修复，300 客户端，15 秒 | 2,163 ms | 1,309 ms | 延迟目标失败 |
| 再修复心跳全场扫描，300 客户端，15 秒 | 1,591 ms | 876 ms | 通过，共 4,236 次交换 |
| 500 客户端，60 秒 | 3,311 ms | 1,854 ms | 14,448 次交换、逐机回执及重连取消通过 |

各次日志位于 `artifacts/pre-field/control-load-*.txt`。500 客户端的 60 分钟持续运行、内存采样和 300 座位真实浏览器更新仍按验证计划推进。原 VM 的 WSS 超时与上述容量开销的关联尚未确定，首次实机测试继续采集客户端、Nginx 和服务端时序。

随后按部署使用的 release 构建进行 60 秒采样，服务端和客户端均为测试进程中的真实网络连接，使用独立测试数据库。Linux 为现有 Ubuntu 24.04.2 / PostgreSQL 16 服务器，内核 `7.0.0-42-generic`，连接使用回环 WS。

| 环境 | 客户端 | 命令接收 p95 | 心跳往返 p95 | 交换次数 |
| --- | --- | --- | --- | --- |
| Windows release | 300 | 984 ms | 211 ms | 16,800 |
| Linux debug | 300 | 2,078 ms，目标失败 | 2,709 ms | 8,802 |
| Linux release | 300 | 1,304 ms | 440 ms | 17,333 |
| Linux release | 500 | 1,890 ms | 914 ms | 21,771 |

Linux 首次 500 客户端测试触发 SSH 会话的 1,024 文件句柄上限。该测试进程同时持有客户端和服务端套接字，在测试子进程内设为 4,096 后通过。新增 Linux 启动前检查，低上限现在立即报告明确条件，日志 `pre-field-limit-preflight.txt` 保存预期失败，`pre-field-limit-retry.txt` 保存提高上限后的短时回归。

## 重跑入口

常规组合检查继续执行 `scripts/check-m2.ps1` 和 Linux 的 `scripts/check-linux.py`。性能用例单独运行，避免与其他综合测试竞争 CPU 和磁盘影响采样。

```powershell
$env:WCCS_TEST_DATABASE_URL = (Get-Content local/m2/server.json -Raw | ConvertFrom-Json).database_url
$env:WCCS_LOAD_CLIENTS = '300'
$env:WCCS_LOAD_SECONDS = '60'
cargo test --release --locked -p wccs-server --target x86_64-pc-windows-msvc --test http concurrent_control_preview_commit_and_reconnect -- --ignored --nocapture
```

Linux 将目标替换为 `x86_64-unknown-linux-gnu`，数据库变量指向专用测试库。客户端数量允许 1～500，采样窗口允许 5～3600 秒。每次测试创建随机 schema，正常结束删除对应 schema。

```bash
export WCCS_TEST_DATABASE_URL="host=/var/run/postgresql user=sustech dbname=wccs_test connect_timeout=5"
(ulimit -n 4096; WCCS_LOAD_CLIENTS=500 WCCS_LOAD_SECONDS=60 cargo test --release --locked -p wccs-server --target x86_64-unknown-linux-gnu --test http concurrent_control_preview_commit_and_reconnect -- --ignored --nocapture)
```

## 首次实机重点与后续顺序

1. 整理可重复安装、逐机身份初始化、机器 CA、版本校验、升级回退及本机恢复入口，先完成参数和失败路径回归。现有登录安装工具包含旧测试机名称约束，需要在该切片改为现场显式机器身份校验。
2. 继续完成比赛流程、compilecpp/submit 和 DOMjudge 接口闭环，以合成数据覆盖超时、重试、重复提交和换机。需要实际 DOMjudge 环境的项目记录实例条件。
3. 现场优先在代表性机器核对账户登录与教学恢复、Windows 热重启、长期等待与 WSS、多屏/RDP、复原软件和卷身份。保存故障日志与复现步骤，离场前核对教学状态恢复。

## 组合检查与部署

- `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` 通过固定 MSVC/SDK、release、fmt、Clippy、83 项 Rust 常规、14 项 PostgreSQL 综合、Credential Provider、36 项前端及生产构建，日志 `artifacts/pre-field/check-m2.txt`。
- Linux `CARGO_BUILD_JOBS=4 python3 scripts/check-linux.py` 通过 release、fmt、Clippy、29 项常规及 14 项 PostgreSQL 综合，日志 `artifacts/pre-field/pre-field-check-linux.txt`。新增句柄预检后再次通过 Linux fmt/Clippy，并检查低上限失败和提高上限成功路径。
- 16 项 Python 回归通过，工作区 206 文件和工作区/暂存区 diff 检查通过。
- Linux 测试服务已切换到 `da43fab-prefield-1113`，二进制 SHA-256 为 `8ebbbf9d9bd91c8ef688184962cc7bb7ac05c0433fd709f5fa5d4c929aecc44c`，数据库 schema 12。部署前备份到 `/var/backups/wccs/pre-field-1113/`，旧 release `da43fab-linux1` 可用于回退。实际脚本保存在 `local/environment/deploy-pre-field.sh`。
- 部署后 WCCS/Nginx/PostgreSQL 为 active，HTTPS 证书验证和管理员 API 登录通过，2 台设备、1 场比赛、2 项等待历史和 10 项维护历史可查，虚拟机设备保持 revoked。证据 `artifacts/pre-field/deployed-api-smoke.json` 和 `pre-field-deploy.txt`。

收尾停止本轮启动的 Windows 独立 PostgreSQL 测试集群。Linux 句柄上限失败测试遗留的唯一 `test_database_` schema 在核对归属和 500 台合成设备后清理，对应前缀剩余数量为 0，命令结果保存于 `pre-field-schema-cleanup.txt`。Linux 测试管理服务继续运行。

Git 改动保存在当前工作区，提交和推送按用户后续要求执行。
