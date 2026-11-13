import { z } from "zod";
import type { Device } from "./contracts";

const unknown = z.object({
  status: z.literal("unavailable"),
  reason: z.string(),
  code: z.number().nullable(),
});
function sample<T extends z.ZodType>(value: T) {
  return z.object({
    sampled_at: z.number(),
    observation: z.union([
      z.object({ status: z.literal("available"), value }),
      unknown,
    ]),
  });
}
export const healthSchema = z.object({
  schema_version: z.literal(1),
  device_id: z.string(),
  instance_id: z.string(),
  sequence: z.number(),
  collection_ms: z.number(),
  agent_version: sample(z.string()),
  deployment_version: sample(z.string()),
  health: z.object({
    uptime_seconds: sample(z.number()),
    cpu_percent: sample(z.number().min(0).max(100)),
    memory: sample(
      z.object({
        total_bytes: z.number(),
        available_bytes: z.number(),
        used_percent: z.number().min(0).max(100),
      }),
    ),
    displays: sample(
      z.array(
        z.object({
          name: z.string(),
          description: z.string(),
          width: z.number(),
          height: z.number(),
          refresh_hz: z.number(),
        }),
      ),
    ),
    keyboards: sample(z.number()),
    mice: sample(z.number()),
  }),
});
export function healthStale(device: Device, now: number) {
  return (
    !device.online ||
    device.health_received_at === null ||
    now - device.health_received_at > 90
  );
}
export function healthAbnormal(device: Device) {
  const h = device.health?.health;
  if (!h) return true;
  return (
    Object.values(h).some((s) => s.observation.status !== "available") ||
    (h.cpu_percent.observation.status === "available" &&
      h.cpu_percent.observation.value >= 95) ||
    (h.memory.observation.status === "available" &&
      h.memory.observation.value.used_percent >= 95) ||
    [h.keyboards, h.mice].some(
      (s) => s.observation.status === "available" && s.observation.value === 0,
    ) ||
    (h.displays.observation.status === "available" &&
      !h.displays.observation.value.length)
  );
}
export function versionMismatch(
  device: Device,
  agent: string,
  deployment: string,
) {
  const current = device.health;
  return Boolean(
    (agent &&
      (current?.agent_version.observation.status !== "available" ||
        current.agent_version.observation.value !== agent)) ||
    (deployment &&
      (current?.deployment_version.observation.status !== "available" ||
        current.deployment_version.observation.value !== deployment)),
  );
}
