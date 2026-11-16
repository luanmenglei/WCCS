# Linux 原生部署与 HTTPS/WSS 联调

## 首台实机联调收尾（2026-11-16）

首台物理客户端完成注册及 HTTPS/WSS/IPC 诊断，控制与三项维护取得实机回执。按用户要求使用独立 nft 表仅对目标来源 IP 的 TCP 443 静默丢包，提前安排 60 秒自动删除。测试后 nft 表清单与原记录完全一致，清理 timer/service 已回收，wccs/nginx/postgresql 均 active。设备重新在线并上报教学恢复，注册窗口关闭。现保留合成单座位比赛、工具配置 4 和维护 profile 9，正式编译器与提交仍待配置验证。详细请求、证据及最终收尾见 [实机交接](offline-client.md#本轮最终诊断与未固化镜像交接2026-11-16)。

## HTTP 入口与 Hydro 停用修复（2026-11-14）

用户反馈直接访问服务器 IP 仍显示 Hydro。实际 HTTP 80 由 root PM2 管理的 Caddy 转发到 Hydro 的回环 8888，HTTPS 443 由 Nginx 转发到 WCCS 的回环 8787。PM2 日志确认 11 月 13 日 11:57 已成功停止 Hydro 相关四个进程，但 14:01 整机启动时，启用的 `pm2-root.service` 执行 `pm2 resurrect`，从仍记录 online 的旧清单恢复了全部进程。

用户授权处理后，先核对 root PM2 仅管理 `hydrooj`、`hydro-sandbox`、`caddy`、`mongodb`，再将两份恢复清单、Caddy 配置、PM2 service 和 Nginx 配置备份到服务器 `/var/backups/wccs/hydro-retirement-20261114-214332/`，目录 0700、文件 0600。

- 以 root、`PM2_HOME=/root/.pm2` 执行 `/root/.nix-profile/bin/pm2 delete <name>`，按上述四个名称逐个停止并移除 PM2 注册。
- 执行两次 `pm2 save --force`，核对 `dump.pm2` 与 `dump.pm2.bak` 均为空数组，再执行 `systemctl disable --now pm2-root.service`。
- 同步 Nginx 部署模板和服务器配置，HTTP 80 使用 308 跳转到固定配置的 HTTPS 主机，并保留请求路径与查询参数。`nginx -t` 通过后执行 `systemctl reload nginx`。
- `systemctl is-active/is-enabled` 核对 PM2 为 inactive/disabled，WCCS、Nginx、PostgreSQL 均为 active/enabled。`ss -ltnp` 核对 80/443 由 Nginx 监听、8787 由 WCCS 监听，8888/27017/5050 已释放；进程检查确认 Hydro、Caddy、MongoDB 与 PM2 已退出。
- Windows Python 使用部署 CA 和禁用代理的 urllib 客户端，验证 HTTP 跟随跳转后为 HTTPS WCCS 页面、状态 200，未登录 `/api/state` 返回 401。`curl.exe --noproxy '*' -I` 核对带路径和查询参数的 HTTP 请求返回相应 HTTPS Location。

自启配置和当前进程状态已实测，整机重启后的复核安排在下次维护窗口。恢复 Hydro 时先协调 HTTP 80 的归属，再使用受保护备份恢复 PM2 清单及服务，并核对原配置和数据库目录。

## 初次部署与后续发布记录

2026-11-13 离线客户端切片再次核对当前 release、schema、三个服务的 active/enabled 和证书有效期，Windows 静态 CRT 客户端与 Linux 完成 HTTPS/HTTP/WSS/原生 IPC 诊断。机器 CA 安装由现场包的显式目标流程执行，详见 [离线客户端交付](offline-client.md)。

2026-11-13 后续完成多客户端控制性能修复，当前测试 release 为 `da43fab-prefield-1113`，schema 12。发布前备份、二进制摘要及 Linux 300/500 客户端采样见 [现场前加固记录](pre-field-hardening.md)。下文保留初次部署命令和验收过程。

日期为 2026-11-13。分支为 `feat/linux-deployment`，应用源码基线为 `da43fab`。用户提供了可通过 SSH 访问的内网测试服务器并授权推进本切片。连接信息和登录材料保存在忽略目录 `local/linux-deployment/`，仓库文档使用部署变量。

## 环境与部署结构

实际目标为 Ubuntu 24.04.2 LTS、x86_64、8 个逻辑 CPU、约 16 GiB 内存。安装 Rust 1.99.1 与 rustfmt/Clippy、GCC 13.3.0、PostgreSQL 16.15、Nginx 1.24.0。Node.js 在 Windows 构建机运行，生产静态资源随服务端复制。Rust 依赖沿用 Cargo.lock，前端沿用 package-lock.json。

| 位置或入口 | 用途 |
| --- | --- |
| `/opt/wccs/releases/<release>` | root 持有的服务二进制与 `web/` 静态资源 |
| `/opt/wccs/current` | 原子切换的当前发布链接 |
| `/etc/wccs/server.json` | root:wccs、0640，配置目录 0750 |
| `/etc/wccs/admin-access.json` | root、0600，首次生成的管理登录材料 |
| `/etc/wccs/tls/` | root、0700，测试 CA 与服务证书 |
| PostgreSQL `wccs` | 部署数据库，由专用 `wccs` 角色持有 |
| PostgreSQL `wccs_test` | 原生集成测试数据库，测试用户持有 |
| `/var/backups/wccs/` | root、0700，数据库及配置备份 |
| `wccs.service` | 非 root 的 `wccs` 系统账户运行，已启用开机启动 |
| `127.0.0.1:8787` | Axum 上游，静态资源、管理 API 与控制连接同源 |
| TCP 80 | Nginx 308 跳转到配置的 HTTPS 地址，保留路径和查询参数 |
| TCP 443 | Nginx TLS 入口，转发 HTTPS 与 WSS |

服务定义使用 SIGINT 对应现有 Rust 优雅退出入口，并设置失败重启、只读系统目录和受限地址族。配置模板位于 [wccs.service](../../config/linux/wccs.service)。Type=exec 的启动检查及重启行为依据 systemd 文档，实际 readiness 另用 HTTP 请求核对。[systemd 服务文档](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html)

Nginx 显式使用 HTTP/1.1、Upgrade 和 Connection 转发 WebSocket。`X-Forwarded-For` 覆盖为当前连接来源，服务端只信任回环代理。模板位于 [nginx.conf.in](../../config/linux/nginx.conf.in)。[Nginx WebSocket 文档](https://nginx.org/en/docs/http/websocket.html)

## 从源码构建

在独立 Linux 源码目录安装仓库固定工具链，准备 C 编译器及 PostgreSQL。测试角色须能在专用测试数据库创建 schema。本轮使用本地 Unix socket 和 peer 认证，部署数据库与测试数据库分开。

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends build-essential pkg-config ca-certificates postgresql nginx
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o rustup-init.sh
sh rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain 1.99.1 --component rustfmt,clippy
. "$HOME/.cargo/env"
sudo -u postgres createuser "$USER"
sudo -u postgres createdb -O "$USER" wccs_test
export WCCS_TEST_DATABASE_URL="host=/var/run/postgresql user=$USER dbname=wccs_test connect_timeout=5"
CARGO_BUILD_JOBS=4 python3 scripts/check-linux.py
```

`createuser` 和 `createdb` 对应首次准备。重复执行时先通过 PostgreSQL 目录核对已有角色和数据库归属。源码使用 `git archive` 导出，可在构建目录解压。前端在已固定 Node.js/npm 的构建机执行 `npm ci` 与 `npm run check`，复制 `web/dist`，包括离线字体。

## 首次安装

以下命令在 Linux 源码根目录执行。`SERVER_IP` 由部署方填写实际地址，示例使用保留文档地址。先核对目标目录、服务名、角色和数据库是否已存在，已有部署按后面的发布切换流程更新。

```bash
SERVER_IP=192.0.2.10
RELEASE=baseline-linux1
sudo useradd --system --home-dir /nonexistent --shell /usr/sbin/nologin wccs
sudo -u postgres createuser wccs
sudo -u postgres createdb -O wccs wccs
sudo install -d -m 0755 /opt/wccs /opt/wccs/releases "/opt/wccs/releases/$RELEASE"
sudo install -m 0755 target/x86_64-unknown-linux-gnu/release/wccs-server "/opt/wccs/releases/$RELEASE/wccs-server"
sudo cp -a web/dist "/opt/wccs/releases/$RELEASE/web"
sudo chown -R root:root "/opt/wccs/releases/$RELEASE"
sudo find "/opt/wccs/releases/$RELEASE/web" -type d -exec chmod 0755 '{}' +
sudo find "/opt/wccs/releases/$RELEASE/web" -type f -exec chmod 0644 '{}' +
sudo ln -s "/opt/wccs/releases/$RELEASE" /opt/wccs/current
sudo install -d -o root -g wccs -m 0750 /etc/wccs
sudo sh -c 'umask 077; /opt/wccs/current/wccs-server --init /etc/wccs/server.json'
sudo python3 - "$SERVER_IP" <<'PY'
import json
from pathlib import Path
import sys
path = Path('/etc/wccs/server.json')
config = json.loads(path.read_text())
config.update(
    origin='https://' + sys.argv[1],
    database_url="host=/var/run/postgresql user=wccs dbname=wccs connect_timeout=5 options='-c statement_timeout=10000'",
    static_dir='/opt/wccs/current/web',
    trusted_proxies=['127.0.0.1'],
)
path.write_text(json.dumps(config, indent=2) + '\n')
PY
sudo chown root:wccs /etc/wccs/server.json
sudo chmod 0640 /etc/wccs/server.json
sudo install -m 0644 config/linux/wccs.service /etc/systemd/system/wccs.service
sed "s/@SERVER_NAME@/$SERVER_IP/g" config/linux/nginx.conf.in | sudo tee /etc/nginx/conf.d/wccs.conf >/dev/null
```

为服务器地址签发含正确 SAN、serverAuth 用途的证书，将证书和私钥分别安装为 `/etc/wccs/tls/server.crt` 与 `server.key`，私钥仅 root 可读。部署浏览器与 Windows 服务账户需要能验证签发链。用户提供的正式信任链可替换本轮测试链。

本轮创建了 90 天测试 CA 和 30 天 IP 服务器证书。服务器证书于北京时间 2026-12-13 12:28:25 到期，CA 于 2027-02-11 12:28:25 到期。Windows 开发机仅在当前用户 Root 证书存储中导入测试 CA，指纹及证书副本保存在 `local/linux-deployment/`。正式 LocalSystem 客户端的机器级信任仍需在独立 Windows 测试目标安装与核验。

```bash
sudo nginx -t
sudo systemd-analyze verify /etc/systemd/system/wccs.service
sudo systemctl daemon-reload
sudo systemctl enable --now wccs
sudo systemctl reload nginx
sudo systemctl status wccs --no-pager
curl --cacert /path/to/ca.crt "https://$SERVER_IP/"
```

本轮首次安装受进程 umask 影响，发布目录及配置目录误设为 0700，服务日志分别报告 CHDIR 和 PermissionDenied。已改为发布目录 0755、配置目录 root:wccs 0750，并在触发启动频率限制后执行 `systemctl reset-failed wccs`。上述安装命令使用显式 `install -m` 固定权限。HTTP 就绪和真实登录均已复核。

## 备份、发布切换与恢复

每次升级前记录当前 release、二进制 SHA-256 与 schema_version，并创建 PostgreSQL 自包含备份及配置备份。配置备份包含管理材料和 TLS 私钥，使用 root 专用目录。

```bash
sudo install -d -m 0700 /var/backups/wccs
sudo sh -c 'umask 077; runuser -u postgres -- pg_dump -Fc wccs > /var/backups/wccs/before-upgrade.dump'
sudo sh -c 'umask 077; tar -czf /var/backups/wccs/before-upgrade-config.tar.gz -C /etc wccs'
readlink /opt/wccs/current
sha256sum /opt/wccs/current/wccs-server
sudo -u postgres psql -d wccs -c 'SELECT version FROM schema_version'
```

安装新版本到独立 release 目录，核对构建结果、依赖、配置兼容性和目录权限，随后停服务并切换链接。发布和回退由单个运维操作串行执行。

```bash
NEXT_RELEASE=/opt/wccs/releases/next-release
sudo systemctl stop wccs
sudo ln -s "$NEXT_RELEASE" /opt/wccs/current.next
sudo mv -Tf /opt/wccs/current.next /opt/wccs/current
sudo systemctl start wccs
```

服务启动后核对 HTTPS、管理登录、数据和客户端重连。回退时将 `NEXT_RELEASE` 设为记录的旧目录。本轮验证同一二进制的二次发布目录与回切，数据库版本保持 12，设备、机房、比赛及管理登录保留。涉及 schema 升级的历史二进制回退须核对其支持版本，必要时使用配套数据库与配置备份恢复。

备份实测恢复到独立 `wccs_restore_check` 数据库，使用临时回环 8788 服务验证管理登录、1 台合成设备、1 间机房和 1 场比赛。探针已停止，数据库保留供复核。

```bash
sudo -u postgres createdb -O wccs wccs_restore_check
sudo sh -c 'runuser -u postgres -- pg_restore --exit-on-error -d wccs_restore_check < /var/backups/wccs/before-upgrade.dump'
```

临时验证配置复制现有配置，明确修改数据库名、回环监听地址和 HTTP origin，随后通过原管理登录/API 验证恢复的数据。测试完成停止临时实例，保留主服务。

## 本轮证据

| 检查或场景 | 实际结果 |
| --- | --- |
| `python3 scripts/check-linux.py`，含测试数据库变量 | 原生 release、rustfmt、Clippy、29 项常规测试、14 项 PostgreSQL HTTP/WebSocket 综合测试通过 |
| Windows 构建机 `npm run check` | 类型、lint、格式、36 项前端测试与生产构建通过 |
| `nginx -t`、`systemd-analyze verify` | 通过，systemd 以 `wccs` 用户运行并启用开机启动 |
| Windows HTTPS 与真实后台浏览器 | 证书验证、静态资源、管理员登录、比赛和座位选择通过 |
| Windows 原生 agent，显式 `--test-mode` | HTTPS 注册、工具检查、健康上报、WSS 与鉴权 IPC 成功 |
| 浏览器进入与结束等待 | 合成身份原生窗口实屏核对，逐机完成回执与普通桌面通过 |
| 等待期间停止 Linux 服务 | 原生窗口退出，浏览器显示过期并暂停写入 |
| 恢复 Linux 服务 | 旧授权取消、客户端保持普通桌面，管理员按会话失效提示重新登录 |
| 停止并恢复 PostgreSQL | 服务 PID 保持，原已登录 API 会话先收到 503，随后恢复 200，25 个采样有记录 |
| 来源地址 | Nginx 保留实际来源；注入伪造 X-Forwarded-For 后仍得到原实际地址；本机虚拟网卡上报与服务端观察分别展示 |
| TLS/鉴权负向检查 | 持正确 CA 访问错误主机地址被拒绝，curl 返回 60；未登录访问 `/api/state` 返回 401 |
| 二次发布与回切 | 原子链接切换、重启与数据保留通过，当前回到首个 Linux release |
| 数据库备份恢复 | 独立数据库恢复及真实服务/API 登录查询通过 |
| 仓库检查 | `python scripts/check_repo.py` 的 200 文件检查、16 项 Python 回归、工作区与暂存区 diff 检查通过 |

证据位于 `artifacts/linux-deployment/`、`artifacts/linux-disconnect.json`、`artifacts/linux-recovery-browser.jpg` 和 `local/linux-deployment/`。服务端构建和安装日志位于 SSH 用户的 `wccs-test/logs/`。测试数据均为合成内容。

## 保留状态与下一步

初次部署结束时保留 HTTPS 管理入口、运行中的 WCCS/Nginx/PostgreSQL、版本目录、配置、备份与恢复验证数据库，注册窗口关闭，开发机测试宿主停止。同日后续已执行整台服务器重启：WCCS、Nginx、PostgreSQL 自动恢复，客户端重连，旧授权失效，浏览器重新登录后历史保留。重启后内核为已安装的 `7.0.0-42-generic`，二进制版本不变。证据及当前状态见 [独立 Windows 10 原生联调](native-windows-linux.md)。

本切片完成 Linux 原生和当前用户 Windows 宿主的跨机闭环。同日后续已覆盖独立 Windows 10 LocalSystem 机器证书信任、SCM/Session 0、普通比赛账户即时登录、维护模板和文件故障恢复；长期图形/连接稳定性等剩余项见上述联调记录。后续在同一 Linux 目标接入 200～300 台控制续租、500 模拟客户端、300 座位 UI 和真实 DOMjudge 测试实例。现场镜像、复原软件、多屏/RDP 继续保留原验收范围。
