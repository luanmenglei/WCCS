import { z } from "zod";
import { rosterSchema } from "./roster-model";
import { seatingSchema } from "./seating";
import { healthSchema } from "./health-model";
import { diagnosticSchema } from "./diagnostics-model";

export const toolSchema = z.object({
  id: z.string(),
  name: z.string(),
  kind: z.enum(["executable", "script"]),
  path: z.string().nullable(),
});
const reason = z.enum([
  "permission_denied",
  "missing",
  "not_found_in_search_scope",
  "not_applicable",
  "not_configured",
  "not_a_file",
  "unsupported",
  "system_error",
]);
const unavailable = z.object({
  status: z.literal("unavailable"),
  reason,
  code: z.number().nullable(),
});
const version = z.union([
  z.object({ status: z.literal("available"), value: z.string() }),
  unavailable,
]);
export const checkSchema = z.union([
  z.object({
    status: z.literal("available"),
    value: z.object({ file_version: version }),
  }),
  unavailable,
]);
export const resultSchema = z.object({
  protocol_version: z.literal(1),
  device_id: z.string(),
  job_id: z.string(),
  config_version: z.string(),
  boot_id: z.string(),
  tools: z.array(
    toolSchema
      .extend({ check: checkSchema })
      .transform(({ check, ...tool }) => ({ tool, check })),
  ),
});
export const deviceSchema = z.object({
  health: healthSchema.nullable().default(null),
  health_received_at: z.number().nullable().default(null),
  diagnostics: diagnosticSchema.nullable().default(null),
  diagnostics_received_at: z.number().nullable().default(null),
  lifecycle: z.enum([
    "pending",
    "enabled",
    "disabled",
    "revoked",
    "rejected",
    "expired",
  ]),
  lifecycle_version: z.number().int(),
  agent_version: z.string().nullable().default(null),
  application_id: z.string().nullable(),
  application_at: z.number().nullable(),
  enrollment_expires: z.number(),
  enrollment_generation: z.number().int(),
  rotation_id: z.string().nullable(),
  rotation_expires: z.number().nullable(),
  id: z.string(),
  name: z.string(),
  registered: z.boolean(),
  online: z.boolean(),
  last_seen: z.number().nullable(),
  desired_job: z.string().nullable(),
  desired_version: z.number().nullable(),
  expired: z.boolean(),
  stale: z.boolean(),
  result: resultSchema.nullable(),
  received_at: z.number().nullable(),
});
export const stateSchema = z.object({
  roster: rosterSchema.default({ version: 0, contests: [] }),
  registration_window: z.object({
    version: z.number().int(),
    closes_at: z.number(),
    open: z.boolean(),
  }),
  seating: seatingSchema.default({ rooms: [], devices: [] }),
  protocol_version: z.literal(1),
  server_time: z.number(),
  version: z.number().int(),
  configuration: z.object({
    schema_version: z.literal(1),
    config_version: z.string(),
    tools: z.array(toolSchema),
  }),
  targets: z.array(z.string()),
  catalog: z.array(toolSchema),
  devices: z.array(deviceSchema),
});
export const sessionSchema = z.object({
  csrf: z.string(),
  id: z.string(),
  username: z.string(),
  display_name: z.string(),
  role: z.enum(["admin", "operator", "viewer"]),
  all_rooms: z.boolean(),
  room_ids: z.array(z.string()),
  version: z.number().int(),
});
export type Tool = z.infer<typeof toolSchema>;
export type Device = z.infer<typeof deviceSchema>;
export type Snapshot = z.infer<typeof stateSchema>;
export type Session = z.infer<typeof sessionSchema>;
export type Check = z.infer<typeof checkSchema>;

export const reasons: Record<z.infer<typeof reason>, string> = {
  permission_denied: "访问被拒绝",
  missing: "文件缺失",
  not_found_in_search_scope: "历史搜索未找到",
  not_applicable: "不适用",
  not_configured: "未配置",
  not_a_file: "路径指向目录",
  unsupported: "不支持",
  system_error: "系统错误",
};
export function checkLabel(check: Check): string {
  return check.status === "available"
    ? "文件存在"
    : `${reasons[check.reason]} (${check.reason})${check.code === null ? "" : ` · ${check.code}`}`;
}
