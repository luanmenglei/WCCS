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
  action: z.enum(["enter", "leave"]),
  expires_at: z.number(),
  identity: identitySchema,
});
const actualSchema = z.object({
  state: z.enum(["desktop", "waiting", "unknown", "failed", "no_session"]),
  command_id: z.string().nullable(),
  session_key: z.string().nullable(),
  monitors: z.number(),
  detail: z.string(),
});
export const waitingPreviewSchema = z.object({
  preview_token: z.string(),
  contest_id: z.string(),
  contest_name: z.string(),
  contest_version: z.number(),
  action: z.enum(["enter", "leave"]),
  duration_seconds: z.number(),
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
      ]),
      device_id: z.string().nullable(),
      identity: identitySchema.nullable(),
      generation: z.number().nullable(),
      instance_id: z.string().nullable(),
      session_key: z.string().nullable(),
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
  ready: "可执行",
  unbound: "未匹配设备",
  conflict: "地址冲突",
  offline: "控制连接离线",
  no_session: "无可用用户会话",
  no_participant: "未分配选手",
  disabled: "设备禁用",
  other_contest: "正在等待另一场比赛",
};
export function actualLabel(item: WaitingItem) {
  if (!item.online || !item.actual) return "离线 · 实际状态未知";
  const labels = {
    waiting: "等待界面已显示",
    desktop: "普通桌面",
    unknown: "状态未知",
    failed: "执行或恢复失败",
    no_session: "无用户会话",
  };
  return labels[item.actual.state];
}
