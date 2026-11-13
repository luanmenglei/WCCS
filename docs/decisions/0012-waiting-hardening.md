# ADR 0012 等待界面防绕过与策略恢复

状态为 Windows 10 独立虚拟机测试实现，2026-11-12。实测矩阵与部署条件见 [任务记录](../tasks/m2-waiting-hardening.md)。沿用 ADR 0011 的在线授权和离线教学恢复。

## 决定

正式等待要求受控普通比赛账户的 SID、活动会话与桌面回执一致。开发预览及前台 test 模式保留退出按钮、Esc、Alt+F4。正式窗口拒绝 WM_CLOSE、退出按钮消息和普通 WM_QUIT，每个显示器使用置顶窗口，定时重新核对前台。正式等待安装本进程低级键盘钩子，仅聚合触碰通知，等待期间吞掉普通桌面的键盘动作。钩子随窗口进程退出释放。

服务在启动窗口前，为目标 SID 设置用户策略 DisableTaskMgr。使用独立受保护文件 device.waiting-policy.json，预写 MachineGuid、SID、原值是否存在、原类型和原始字节，再写注册表并 flush。恢复只接受本轮值或原值，发现外部冲突保留日志并报错。原值缺失时删除该值，恢复后的空容器键可以保留。已卸载用户配置单元通过 ProfileList 的 NTUSER.DAT 与 RegLoadKey 打开，在专用恢复挂载名下操作，确认卸载后清除日志。备份/还原权限使用 RAII 恢复原状态，加载冲突保留日志重试。RegLoadAppKey 要求配置单元内部采用相同安全描述符，因此最终实现采用支持用户配置单元不同 ACL 的 RegLoadKey。[配置单元 API 约束](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regloadappkeyw)

等待结束后先确认窗口退出与策略恢复，再回传完成。窗口异常、会话变化、连接明确断开和单调心跳超时均结束本轮授权，并撤回等待策略与账户限制。保留原成功回执和当前失败观察。自动重新创建受控窗口需要新的管理操作。服务启动先恢复残留，恢复失败阻止新启用并持续回传原因。独立 Recover 入口使用同一身份锁和恢复逻辑。

保存权限原状态时同时提供 AdjustTokenPrivileges 的 PreviousState 与 ReturnLength 缓冲区。Windows 10 冷启动实测曾暴露遗漏 ReturnLength 导致 ERROR_NOACCESS，现已修复并加入普通令牌可执行的原生调用回归，未加载用户配置单元恢复结果另行记录。[权限调整 API](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-adjusttokenprivileges)

当前协议维持 v1 的字段结构，新增固定 detail 值。管理页面使用中文说明，旧 detail 保持可显示。部署需配套更新服务端和客户端，旧服务端会拒绝新增 detail。未增加第三方依赖。

## 权限与会话边界

SYSTEM 创建的等待进程保留 SYSTEM 所有者、受保护 DACL、启动时 DLL 缓解和鉴权管道。普通比赛账户的结束进程、服务控制与策略修改实际拒绝结果单独记录。管理员具有调试权限时可以结束进程，公开教学账户禁用也不能撤销已有管理员令牌。正式启用继续依赖账户切片的会话与权限核验。[进程访问权限](https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights)

Ctrl+Alt+Del 的安全桌面仍由 Windows 处理。任务管理器策略移除其对应入口，锁定、切换用户、注销等其他 Windows 操作保留。锁定或切换导致会话失效时按恢复模型撤回控制。主动断网同样会触发撤回，重新连接须重新授权。这些路径属于当前产品恢复取舍，不能宣称客户端具备抵抗本机管理员或所有本地主动干预的能力。[任务管理器策略映射](https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-admx-ctrlaltdel)

GetForegroundWindow 在安全桌面显示期间可能仍返回原普通桌面的窗口，因此等待回执同时核验当前输入桌面。前台被覆盖或输入桌面不可观察时报告 unknown 和 waiting_focus_lost，窗口继续尝试恢复置顶与前台。普通桌面的激活尝试允许短暂 AttachThreadInput，调用后立即解除关联；对端挂死仍由宿主 IPC 超时处理。Windows 的前台限制仍由系统执行。[前台窗口规则](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow) [输入队列关联](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-attachthreadinput)

低级键盘钩子在回调超时后可能被系统静默移除，回调只发送无指针通知，不执行 IO。宿主的 IPC 超时继续检测挂死窗口。管理员注入、辅助技术、触摸与具体机房驱动组合须按现场条件验证。[低级键盘钩子](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)

## 恢复与重新评估

注册表读取与写入无法跨外部管理员实现条件事务。外部修改后又恢复为相同值无法检测，已被管理员篡改的系统可信性继续由部署与现场运维处理。服务连续崩溃并耗尽 SCM 重试时，窗口会因 IPC 断开退出，持久策略和账户限制需要本机恢复入口。

生产采用前需核对真实机房 Windows 版本、恢复管理员图形登录、教学会话退出、复原软件、用户配置单元恢复、多屏混合 DPI 和辅助工具。发现新的恢复失败、普通账户绕过或系统兼容问题时重新评估本方案。
