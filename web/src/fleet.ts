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

export const pages = {
  roster: {
    label: "选手名单",
    group: "赛前准备",
    description: "按比赛导入名单、核对差异并分配或调整座位。",
  },
  seats: {
    label: "座位",
    group: "赛场管理",
    description: "查看机房布局、设备绑定与需要处理的座位。",
  },
  devices: {
    label: "设备",
    group: "赛场管理",
    description: "按连接、绑定和工具检查状态查找设备，查看最新回传。",
  },
  enrollment: {
    label: "注册审核",
    group: "赛前准备",
    description: "管理注册窗口，核对设备申请和重新授权。",
  },
  targets: {
    label: "检查任务",
    group: "赛场管理",
    description: "选择座位，预览实际设备及跳过原因，提交工具检查。",
  },
  rooms: {
    label: "机房设置",
    group: "赛前准备",
    description: "维护机房尺寸、连续编号和预期 IP，预览影响后保存。",
  },
  configuration: {
    label: "工具配置",
    group: "赛前准备",
    description: "维护全局工具路径和完整目标集合，发布版本化检查配置。",
  },
} as const;
export type Page = keyof typeof pages;
export function pageFromHash(hash: string): Page {
  const value = hash.replace(/^#\/?/, "").split("?")[0];
  return Object.hasOwn(pages, value) ? (value as Page) : "seats";
}
