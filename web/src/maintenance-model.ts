import { z } from "zod";

export const maintenanceTemplates: Record<string, string> = {
  recovery: "赛后恢复",
  identify: "设备识别",
  prepare_desktop: "桌面入口",
  check_workspace: "普通账户工作目录检查",
  deploy_scripts: "竞赛脚本包部署",
};

export const maintenanceReasons: Record<string, string> = {
  ordinary_account_unavailable: "普通比赛账户尚未准备或适配器不可用",
  device_disabled: "设备未启用",
  offline: "维护或控制通道离线",
  no_session: "没有活动用户会话",
  unsupported_client: "客户端未声明此模板能力",
  waiting_active: "等待或登录控制正在生效",
  maintenance_active: "已有维护任务",
};
export const maintenanceStages: Record<string, string> = {
  queued: "等待领取",
  accepted: "客户端已持久保存",
  executing: "执行中",
  succeeded: "完成",
  failed: "失败",
  cancelled: "已停止",
  interrupted: "重启中断",
};
export const maintenanceDetails: Record<string, string> = {
  desktop_updated: "桌面入口已更新并核验",
  workspace_verified: "普通账户目录与工具访问核验通过",
  scripts_activated: "脚本版本已校验并激活",
  preparation_started: "正在核验账户、路径和模板条件",
  rollback_complete: "修改已回退，可核对后重新下发",
  rollback_failed: "恢复遇到文件变动或占用，请核对本机事务记录",
  persisted: "任务已持久保存",
  display_observed: "用户会话已确认识别画面可见",
  display_closed: "到期关闭并收到桌面回执",
  cancelled_locally: "本机关闭了识别画面",
  identity_changed: "用户会话或身份已变化",
  connection_lost: "连接或授权结束，画面已停止",
  session_unavailable: "无法取得用户会话回执",
  execution_failed: "执行失败，请导出本机诊断包",
  restart_interrupted: "进程重启，保留中断记录",
};
export const maintenancePreviewSchema = z.object({
  preview_token: z.string(),
  preparation: z
    .object({
      version: z.number(),
      profile: z.object({
        tool_version: z.number(),
        work_directory: z.string(),
        desktop_directory: z.string(),
        script_directory: z.string(),
      }),
      tools: z.array(z.object({ id: z.string(), path: z.string() })),
    })
    .nullable()
    .default(null),
  targets: z.array(
    z.object({
      device_id: z.string(),
      name: z.string(),
      reason: z.string().nullable(),
      adapter: z.string().nullable(),
      identity: z.object({ seat_label: z.string(), school: z.string() }),
    }),
  ),
});
export const maintenanceListSchema = z.object({
  jobs: z.array(
    z.object({
      job_id: z.string(),
      request_id: z.string(),
      device_id: z.string(),
      stage: z.string(),
      expired: z.boolean(),
      cancel_requested: z.boolean(),
      updated_at: z.number(),
      command: z.object({ template: z.object({ operation: z.string() }) }),
      result: z
        .object({
          detail: z.string(),
          steps: z
            .array(
              z.object({
                phase: z.string(),
                path: z.string(),
                outcome: z.string(),
                detail: z.string(),
              }),
            )
            .default([]),
        })
        .nullable(),
    }),
  ),
  server_time: z.number(),
});
