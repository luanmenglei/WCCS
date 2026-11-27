# 代码可维护性重构（2026-11-27）

## 审查判断

部署边界能对应赛场职责。阅读成本主要集中在进程内部。例如，确认一次维护任务会不会重复执行，原先要从一段 400 多行的轮询函数中找出旧回执判断，再绕过关机、恢复和识别窗口的执行分支。管理台有多个页面从座位检查组件导入通用写请求类型，四处代码各自清理过期会话。此次调整覆盖五个 Rust 包、管理台及镜像打包入口。

## 如何读一次修改

- **查维护重复派发**。从 [cycle](../../crates/wccs-agent/src/maintenance.rs) 按顺序读补报、回滚、领取、查旧回执、落盘、执行和确认。需要知道服务器为何仍允许执行时读 [channel](../../crates/wccs-agent/src/maintenance/channel.rs)，需要知道重启后还剩什么证据时读 [journal](../../crates/wccs-agent/src/maintenance/journal.rs)。普通账号准备、恢复及关机继续使用各自的授权条件。
- **查服务启动**。按 [connect](../../crates/wccs-server/src/app.rs) 的事务顺序核对迁移与旧任务退役。21 个已发布 SQL 文件按数据库版本排列，备用机预检和正常启动使用同一目标版本。服务端路由按管理请求和设备请求编排。
- **改窗口和管理台**。窗口文字按具名控件定位，子窗口及字体的释放与创建位于同一模块。管理会话集中处理登录、401、轮询与请求编号。机房选择、草稿保存、逐格编辑、批量 IP 和地址核对分别有入口。[源码定位表](../architecture.md#按维护任务定位代码)
- **改镜像封装**。在 [package-image.py](../../scripts/package-image.py) 中分别查看内层客户端包校验、母机目标与公共配置的转换以及最终组包。选手脚本和原生登录提供程序保留各自的构建入口。

## 验证范围

本地检查使用 Windows 固定工具链、回环 PostgreSQL 18 的隔离 schema、合成设备和临时目录。开发环境的浏览器测试连接本地服务端和生产版静态页面。线上部署、真实机房的登录或关机、长时间容量演练仍按[路线图](../roadmap.md)安排。

| 实际检查 | 核对结果 |
| --- | --- |
| `pwsh -NoProfile -File scripts/check-m2.ps1 -ServerConfig local/m2/server.json` | Windows 五包 release、格式、Clippy、107 项常规 Rust 测试和 Credential Provider 构建通过。26 项 PostgreSQL/HTTP/WSS 回归通过，覆盖从 v1 升级、重启、旧授权退役及回执重试 |
| 同一入口的前端检查及 `npm --prefix web run check` | TypeScript、ESLint、Prettier、38 项测试和生产构建通过。Vite 提示主 JS 包压缩前为 501.10 kB |
| Edge、本地服务与合成比赛 | 300 个座位的机房有未保存修改时，批量检查按钮禁用，12 个批量 IP 依次完成预览、应用和确认，通过 API 读回首尾地址，页面显示已保存版本 v1。主动退出、服务端会话失效、再次登录后清空旧回执并恢复书签比赛通过。换座后刷新失败的提示与评测端跳转也通过 |
| `cargo check --locked -p wccs-server -p wccs-protocol --target x86_64-unknown-linux-gnu` | Windows 上使用 Clang、llvm-ar 和 Debian amd64 头文件的交叉类型检查通过 |
| `python -m unittest discover -s scripts/tests -v` | 28 项通过，含镜像包、竞赛脚本及仓库检查回归 |
| `python scripts/check_repo.py`、`git diff --check`、`git diff --cached --check` | 工作区文件、文档相对链接与差异格式通过 |

单独运行 `cargo test --locked -p wccs-server --target x86_64-pc-windows-msvc --test http -- --ignored concurrent_control_preview_commit_and_reconnect --nocapture`，使用相同的 300 客户端、15 秒参数。首次命令接收 P95 为 2,064 毫秒，超过 2,000 毫秒门槛；复测为 1,916 毫秒，通过。两轮轮询 P95 分别为 1,265 和 984 毫秒，逐机结果、审计与重连断言均通过。容量验收仍需在目标环境重复测量。
