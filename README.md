# Windows Contest Control System

WCCS 服务于 200～300 台 Windows 10 选手机的多机房单人算法竞赛。管理员从浏览器分配座位和选手、准备比赛账户、控制等待与开赛、分发文件并核对赛后恢复。教学期间服务器可以关闭。比赛中失联时，客户端撤回当轮限制，保留当前会话和文件；重连后由管理员重新授权。

Linux 上的 Rust 服务端与 PostgreSQL 保存比赛、设备及操作回执，React 管理台随服务端同源交付。Windows 服务持有控制连接，用户会话程序显示等待界面。客户端通过 WSS 接收控制、通过服务器 HTTPS 下载文件。`compilecpp` 与 `submit` 作为独立脚本包发布。[系统架构](docs/architecture.md)

## 当前状态

开发分支已实现 schema 20、单活动控制器和服务器接管预检。最近一次现场服务端发布记录为 2026-11-18 的 schema 19。新版已在开发机完成双进程接管与浏览器验证，两台 Linux 主机的实际切换、机房镜像重启和全场演练仍待完成。[路线图与交接](docs/roadmap.md)

## 部署与值班

| 要做的事 | 从这里开始 |
| --- | --- |
| 首次部署 Linux 服务、配置 HTTPS、备份数据库 | [Linux 部署说明](docs/tasks/linux-deployment.md) |
| 准备候选服务器、切换、回退或处理入口不可用 | [服务器切换手册](docs/tasks/server-cutover.md) |
| 制作统一镜像、处理克隆后注册和复原 | [母机与镜像部署手册](docs/tasks/image-deployment-field-guide.md) |
| 安装、诊断或恢复一台 Windows 客户端 | [单机现场操作手册](docs/tasks/offline-client-field-guide.md) |
| 配置机房、换座、处理暂停和核对任务回执 | [管理员操作走查](docs/tasks/admin-workflows.md) |
| 联动 DOMjudge、发布资料、归档并恢复教学 | [M3～M5 赛场操作手册](docs/tasks/m3-m5-field-guide.md) |

## 本地开发

在仓库根目录执行。新克隆先运行 `setup.ps1` 安装本地 Git hook。仓库检查使用 Python 3.11+，Windows 构建使用固定的 Rust、MSVC 和 SDK。[开发规范与工具链](docs/development.md)

```powershell
pwsh -NoProfile -File scripts/setup.ps1
python scripts/check_repo.py
python -m unittest discover -s scripts/tests -v
pwsh -NoProfile -File scripts/check-rust.ps1
```

开发管理台时先安装锁定依赖，再运行前端检查。

```powershell
npm --prefix web ci
npm --prefix web run check
```

已准备独立 PostgreSQL 测试库时，运行 `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json`。Linux 原生构建与测试入口为 `python3 scripts/check-linux.py`。本地服务、设备身份和管理台的启动方法见[开发联调说明](docs/tasks/m2-tool-configuration.md)。

## 项目文档

- [需求基线](docs/requirements.md)与[部署与接管决策](docs/decisions/0017-site-deployment-and-cutover.md)记录现场约束、身份信任、授权和恢复规则。
- [管理界面规范](docs/ui-design.md)与[采集工具说明](docs/inventory.md)分别说明浏览器交互和 Windows 环境检查。
- [验证方案](docs/validation.md)列出自动化命令、实机验收条件和两次现场窗口计划；[历史交接](docs/roadmap-history.md)保留早期里程碑与实测证据。
