import type { Device, Snapshot } from "./contracts";

export const seatStatusLabels = {
  online: "在线",
  offline: "离线",
  unbound: "未绑定",
  conflict: "冲突",
};

export function seatRows(snapshot: Snapshot) {
  return snapshot.seating.rooms.flatMap((room) =>
    room.seats.map((seat) => ({ room, seat })),
  );
}

export function checkStatus(device: Device): string {
  if (device.lifecycle === "revoked") return "凭据已撤销";
  if (device.lifecycle === "disabled") return "检查领取已暂停";
  if (!device.registered) return "待注册";
  if (device.expired) return "任务已过期";
  if (!device.result) return device.desired_job ? "等待回传" : "尚未下发";
  if (device.stale) return "结果已过期";
  return device.result.tools.some((item) => item.check.status === "unavailable")
    ? "有检查异常"
    : "检查完成";
}

export function matchesSearch(
  values: (string | number | null | undefined)[],
  query: string,
) {
  const normalized = query.trim().toLocaleLowerCase();
  return (
    !normalized ||
    values.some((value) =>
      String(value ?? "")
        .toLocaleLowerCase()
        .includes(normalized),
    )
  );
}
