# Windows Contest Control System

WCCS 用于 Windows 10 多机房单人算法竞赛，目标规模为 200～300 台选手机。提供座位与选手绑定、比赛等待界面、统一操控、状态监测、文件分发，以及 `compilecpp`、`submit` 等竞赛命令。

日常教学时客户端离线待机。比赛时由服务器的 Web 控制台集中管理。赛后可清空 D 盘并重启，通过机房镜像复原恢复教学环境。

## 当前阶段

项目处于仓库与架构初始化阶段，当前交付包括项目指令、需求基线、设计文档、开发规范和可执行的仓库检查。下一步为只读环境采集及 Windows 技术验证。

## 开发入口

需要 Git、Python 3.11 或更新版本，以及 PowerShell 7。运行以下命令安装本仓库的本地 Git hook 并验证基础设施。

```powershell
pwsh -NoProfile -File scripts/setup.ps1
python scripts/check_repo.py
python -m unittest discover -s scripts/tests -v
```

`setup.ps1` 设置本仓库的换行和 hook 配置。新克隆或新的开发环境需要运行一次。应用工具链版本在首个应用工程落地时固定。

## 文档导航

| 文档 | 用途 |
| --- | --- |
| [AGENTS.md](AGENTS.md) | Codex 工作入口和项目约束 |
| [需求基线](docs/requirements.md) | 已确认需求、范围和现场未知项 |
| [架构](docs/architecture.md) | 组件、状态、身份、协议和部署设计 |
| [架构决策](docs/decisions/0001-project-baseline.md) | 本轮决策及暂定技术选型 |
| [开发规范](docs/development.md) | 代码、Git、Codex 开发与交接流程 |
| [验证方案](docs/validation.md) | 自动化检查、容量目标和真机演练 |
| [路线图](docs/roadmap.md) | 当前状态、任务顺序和完成条件 |

## 从 Codex 开始下一项工作

可以向 Codex 提交以下任务。

> 阅读 AGENTS.md、需求基线和路线图，执行 M1 的只读环境采集切片。实现可导出脱敏结果的采集工具及验证。依据已有证据推进，现场未知项记录下来。完成后更新路线图，并报告实际运行的检查和下一步。

该任务只采集开发机或明确指定测试机的环境。机房真实复原、设备策略和自动登录演练按验证文档在对应环境开展。
