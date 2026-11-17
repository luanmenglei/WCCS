# DOMjudge 9.0.2 暂停联动扩展

已安装到 10.16.37.123 的 DOMjudge 9.0.2，实际 API、重载恢复和 WCCS 管理端联动回归通过。原生 DOMjudge 网页时钟可视验证待补。

控制器复用 DOMjudge 的 Contest、RemovedInterval、提交开关、审计及事件机制。暂停期间关闭提交，使用 2100-01-01 作为未结束区间的内部占位终点，防止 WCCS 服务中断导致自动恢复。恢复时将区间终点替换成实际恢复时刻，顺延暂停后的结束、封榜、解榜及停用时间。已完成的里程碑保持原值。网页时钟通过独立状态接口显示暂停与剩余时间。

写入需要 api_writer 权限、预览版本与配置摘要。相同请求编号持久复用原结果。外部修改导致摘要变化时要求重新核对。新增两张 wccs_ 前缀表随 DOMjudge 数据库备份。卸载前必须恢复或结束所有暂停比赛，防止遗留未结束区间。

扩展文件采用 AGPL-3.0-or-later，依赖 DOMjudge 9.0.2 的同类许可组件。升级 DOMjudge 前需要重新核对实体、路由、提交限制与计分接口，并重新执行暂停前后提交时间与计分回归。[DOMjudge 源码与许可](https://github.com/DOMjudge/domjudge/tree/9.0.2)

安装入口为 install.py，要求指定 domserver、源码 webapp 和备份目录，并检查固定版本及原始文件摘要。contest_timing.py 修正原生 CLICS 时长对暂停区间的计算。开始安排通过 hold_start 禁止提前自动开赛，WCCS 核对桌面后调用 start。取消安排保持等待开赛，需新的明确操作。

暂停时原生 end_time 为内部占位终点，外部系统应读取 wccs-control 的 phase 和 remaining_seconds。恢复时替换成实际时间。完整实现和测试记录见 [M3～M5 开发记录](../../docs/tasks/m3-m5-implementation.md)。
