import { z } from "zod";

export const identitySchema = z.object({
  contest_id: z.string(),
  contest_name: z.string(),
  contest_version: z.number(),
  participant_id: z.string(),
  name: z.string(),
  school: z.string(),
  seat_id: z.string(),
  seat_label: z.string(),
  room_id: z.string(),
  room_version: z.number(),
});
export const accountSchema = z.object({
  policy: z
    .object({
      requested: z.object({
        block_removable_storage: z.boolean(),
        block_shutdown: z.boolean(),
      }),
      state: z.enum(["ready", "applied", "pending", "recovery_failed"]),
    })
    .optional(),
  adapter: z.enum(["unavailable", "simulation", "windows_test"]),
  revision: z.number(),
  phase: z.enum([
    "teaching",
    "preparing",
    "login_pending",
    "controlled",
    "recovering",
    "recovery_failed",
  ]),
  account: z.enum(["unavailable", "absent", "owned", "conflict"]),
  target_sid: z.string().nullable(),
  session: z.string(),
  observed_sid: z.string().nullable(),
  desktop_confirmed: z.boolean(),
  recovery_ready: z.boolean(),
  tools_ready: z.boolean(),
  teaching_enabled: z.boolean(),
  detail: z.string(),
});
export const actionLabels = {
  enter: "进入等待",
  leave: "结束等待",
  prepare_login: "准备并统一登录",
  restore_teaching: "恢复教学账户",
};
export type ControlAction = keyof typeof actionLabels;
const actionSchema = z.enum([
  "enter",
  "leave",
  "prepare_login",
  "restore_teaching",
]);
export function accountLabel(
  account: z.infer<typeof accountSchema> | null | undefined,
) {
  if (!account || account.adapter === "unavailable") return "登录适配尚未就绪";
  const phases = {
    teaching: "教学可用",
    preparing: "准备中",
    login_pending: "等待登录确认",
    controlled: "比赛账户已登录",
    recovering: "恢复中",
    recovery_failed: "恢复失败",
  };
  const policy = account.policy;
  const policyText =
    policy?.state === "applied"
      ? ` · ${policy.requested.block_removable_storage ? "USB 存储受限 " : ""}${policy.requested.block_shutdown ? "软件关机受限" : ""}`
      : policy?.state === "recovery_failed"
        ? " · 策略恢复待处理"
        : "";
  return `${account.adapter === "simulation" ? "模拟" : "Windows 测试"} · ${phases[account.phase]} · ${account.detail}${policyText}`;
}
export const commandSchema = z.object({
  protocol_version: z.literal(1),
  command_id: z.string(),
  request_id: z.string(),
  sequence: z.number(),
  device_id: z.string(),
  generation: z.number(),
  device_version: z.number(),
  instance_id: z.string(),
  session_key: z.string(),
  action: actionSchema,
  expires_at: z.number(),
  identity: identitySchema,
  login: z
    .object({
      restrictions: z
        .object({
          block_removable_storage: z.boolean(),
          block_shutdown: z.boolean(),
        })
        .optional(),
      tool_version: z.number().optional(),
      tool_paths: z.array(z.string()).optional(),
      connection_id: z.string(),
      snapshot: accountSchema,
      confirm_session_end: z.boolean(),
    })
    .optional(),
});
const actualSchema = z.object({
  state: z.enum(["desktop", "waiting", "unknown", "failed", "no_session"]),
  command_id: z.string().nullable(),
  session_key: z.string().nullable(),
  monitors: z.number(),
  detail: z.string(),
  account: accountSchema.optional(),
});
export const waitingPreviewSchema = z.object({
  preview_token: z.string(),
  contest_id: z.string(),
  contest_name: z.string(),
  contest_version: z.number(),
  action: actionSchema,
  duration_seconds: z.number(),
  tool_version: z.number().optional(),
  tool_paths: z.array(z.string()).optional(),
  targets: z.array(
    z.object({
      seat_id: z.string(),
      room_id: z.string(),
      room_name: z.string(),
      number: z.number(),
      status: z.enum([
        "ready",
        "unbound",
        "conflict",
        "offline",
        "no_session",
        "no_participant",
        "disabled",
        "other_contest",
        "login_unavailable",
        "policy_unavailable",
        "recovery_required",
        "account_conflict",
        "recovery_unavailable",
        "tools_unavailable",
        "session_confirmation_required",
        "maintenance_active",
        "unsupported_session",
      ]),
      device_id: z.string().nullable(),
      identity: identitySchema.nullable(),
      generation: z.number().nullable(),
      instance_id: z.string().nullable(),
      session_key: z.string().nullable(),
      account: accountSchema.nullable().optional(),
    }),
  ),
});
export const waitingItemSchema = z.object({
  command: commandSchema,
  stage: z.enum([
    "pending",
    "accepted",
    "executing",
    "succeeded",
    "failed",
    "cancelled",
    "expired",
  ]),
  report: z
    .object({
      command_id: z.string(),
      sequence: z.number(),
      stage: z.string(),
      actual: actualSchema,
    })
    .nullable(),
  online: z.boolean(),
  actual: actualSchema.nullable(),
  last_seen: z.number().nullable(),
  updated_at: z.number(),
});
export const waitingListSchema = z.object({
  items: z.array(waitingItemSchema),
  server_time: z.number(),
});
export type WaitingItem = z.infer<typeof waitingItemSchema>;
export const stageLabels = {
  pending: "等待接收",
  accepted: "设备已接收",
  executing: "执行中",
  succeeded: "已完成",
  failed: "失败",
  cancelled: "已取消",
  expired: "已过期",
};
export const skipLabels = {
  desktop_upgrade_required: "客户端尚未就绪或需要升级",
  desktop_account_required: "请先准备并登录普通比赛账户",
  pause_upgrade_required: "请更新客户端后再执行持续暂停",
  policy_unavailable: "客户端尚不支持比赛策略，请先升级",
  maintenance_active: "维护任务正在执行",
  ready: "可执行",
  unbound: "未匹配设备",
  conflict: "地址冲突",
  offline: "控制连接离线",
  no_session: "无可用用户会话",
  no_participant: "未分配选手",
  disabled: "设备禁用",
  other_contest: "另一场比赛仍在控制",
  login_unavailable: "登录适配尚未就绪",
  recovery_required: "需先完成恢复或登录",
  account_conflict: "账户所有权或权限冲突",
  recovery_unavailable: "离线恢复入口未就绪",
  tools_unavailable: "工具或工作目录访问未通过",
  session_confirmation_required: "需确认当前会话已保存并同意退出",
  unsupported_session: "锁屏、多会话、RDP 或未知会话需现场处理",
};
export function actualLabel(item: WaitingItem) {
  if (!item.online || !item.actual) return "离线 · 实际状态未知";
  if (
    item.actual.state === "waiting" &&
    item.actual.command_id !== item.command.command_id
  )
    return "另一命令的等待界面";
  const labels = {
    waiting: "等待界面已显示",
    desktop: "普通桌面",
    unknown: "状态未知",
    failed: "执行或恢复失败",
    no_session: "无用户会话",
  };
  return labels[item.actual.state];
}

const waitingDetails: Record<string, string> = {
  waiting_focus_lost: "等待窗口失去前台，正在尝试恢复显示",
  waiting_account_required: "需先完成普通比赛账户准备与桌面确认",
  waiting_policy_apply_failed: "任务管理器限制应用失败，已请求撤回",
  waiting_policy_changed: "任务管理器策略被外部修改，已停止等待",
  waiting_policy_recovery_failed:
    "等待策略恢复失败，需核对本机日志并执行恢复工具",
};
export function waitingDetail(detail: string | undefined) {
  return detail ? (waitingDetails[detail] ?? detail) : "尚未回传";
}
