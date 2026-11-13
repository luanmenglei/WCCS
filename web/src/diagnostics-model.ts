import { z } from "zod";

export const diagnosticSteps = {
  configuration: "本机配置",
  installation: "安装状态",
  server: "服务器连接",
  authentication: "注册与认证",
  http_heartbeat: "HTTP 心跳",
  websocket: "WSS 实际回执",
  service: "SCM 服务",
  session_ipc: "会话鉴权 IPC",
};
export const diagnosticOutcomes = {
  succeeded: "通过",
  failed: "失败",
  pending: "待联网验证",
  unknown: "未知",
};
export const diagnosticRecovery = {
  receipt_verified: "已核对实际回执",
  saved_offline: "本地保存成功，可继续执行联网诊断",
  not_configured: "在本机配置入口填写服务器或服务信息",
  invalid_configuration: "刷新配置并重新导入受支持的版本",
  connection_unavailable:
    "教学离线可保持待机，联网后检查地址、证书和端口并重试",
  authentication_rejected: "核对管理端设备状态与凭据，必要时重新授权",
  registration_pending: "到注册审核页核对本设备申请并批准",
  registration_rejected: "重新导出本设备注册材料并核对申请",
  incompatible_reply: "核对客户端与服务端版本",
  service_running: "SCM 确认服务正在运行",
  service_stopped: "使用安装工具核对目标服务后启动",
  service_missing: "使用安装工具将客户端安装到明确的目标机器",
  permission_denied: "使用工作人员账户核对安装目录及服务查询权限",
  session_unavailable: "核对服务对应身份文件、活动控制台与会话程序后重试",
  ipc_failed: "核对会话程序版本和鉴权管道日志",
  file_missing: "核对安装目录中的客户端与会话程序",
};
export const diagnosticSchema = z.object({
  schema_version: z.literal(1),
  request_id: z.string(),
  device_id: z.string(),
  agent_version: z.string(),
  evidence_context: z.enum(["service", "local_test"]),
  configuration_revision: z.number().int(),
  configuration_digest: z.string(),
  server_origin: z.string().nullable().default(null),
  collected_at: z.number(),
  seat_claim: z
    .object({
      room_id: z.string(),
      seat_number: z.number().int(),
      interface_index: z.number().int(),
      mac_address: z.string(),
    })
    .nullable(),
  checks: z.array(
    z.object({
      step: z.enum(
        Object.keys(diagnosticSteps) as [
          keyof typeof diagnosticSteps,
          ...Array<keyof typeof diagnosticSteps>,
        ],
      ),
      outcome: z.enum(
        Object.keys(diagnosticOutcomes) as [
          keyof typeof diagnosticOutcomes,
          ...Array<keyof typeof diagnosticOutcomes>,
        ],
      ),
      checked_at: z.number(),
      last_success_at: z.number().nullable(),
      code: z.enum(
        Object.keys(diagnosticRecovery) as [
          keyof typeof diagnosticRecovery,
          ...Array<keyof typeof diagnosticRecovery>,
        ],
      ),
    }),
  ),
});
