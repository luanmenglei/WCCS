export const pages = {
  seats: {
    label: "座位",
    group: "现场巡查",
    description: "查看机房布局、设备绑定与需要处理的座位。",
  },
  devices: {
    label: "设备",
    group: "现场巡查",
    description: "按连接、绑定和工具检查状态查找设备，查看最新回传。",
  },
  events: {
    label: "全局事件",
    group: "现场巡查",
    description: "按授权范围查询管理动作、设备上报与任务结果。",
  },
  enrollment: {
    label: "注册审核",
    group: "赛前布置",
    description: "管理注册窗口，核对设备申请和重新授权。",
  },
  rooms: {
    label: "机房设置",
    group: "赛前布置",
    description: "维护机房尺寸、连续编号和预期 IP，预览影响后保存。",
  },
  roster: {
    label: "选手名单",
    group: "赛前布置",
    description: "按比赛导入名单、核对差异并分配或调整座位。",
  },
  domjudge: {
    label: "DOMjudge 账号",
    group: "赛前布置",
    description: "核对选手、座位和评测账号，更新绑定并撤销换机前的网页会话。",
  },
  configuration: {
    label: "工具配置",
    group: "赛前布置",
    description: "维护全局工具路径和完整目标集合，发布版本化检查配置。",
  },
  targets: {
    label: "检查任务",
    group: "赛前布置",
    description: "选择座位，预览实际设备及跳过原因，提交工具检查。",
  },
  login: {
    label: "比赛账户与登录",
    group: "赛前布置",
    description: "预览会话、准备比赛账户并查询登录与教学恢复状态。",
  },
  presentation: {
    label: "等待展示",
    group: "赛前布置",
    description:
      "设置开赛时间、DOMjudge 状态来源与学校校徽，查看逐机展示诊断。",
  },
  distribution: {
    label: "文件分发",
    group: "赛前布置",
    description: "上传资料、选择设备、安排发布并核对下载与补发结果。",
  },
  waiting: {
    label: "等待控制",
    group: "比赛与收场",
    description: "预览固定选手与设备，批量进入或结束等待，核对逐机实际状态。",
  },
  "contest-control": {
    label: "比赛安排",
    group: "比赛与收场",
    description: "安排开始、暂停、恢复与结束，核对 DOMjudge 和桌面的联动结果。",
  },
  recovery: {
    label: "赛后恢复",
    group: "比赛与收场",
    description: "核对归档范围，执行清理、重启并确认教学恢复。",
  },
  maintenance: {
    label: "批量维护",
    group: "系统管理",
    description: "预览固定操作模板、显示设备识别并核对逐机执行结果。",
  },
  operators: {
    label: "操作员",
    group: "系统管理",
    description: "创建账号、核对权限并管理账号状态与登录凭据。",
  },
} as const;
export type Page = keyof typeof pages;
export function pageFromHash(hash: string): Page {
  const value = hash.replace(/^#\/?/, "").split("?")[0];
  return Object.hasOwn(pages, value) ? (value as Page) : "seats";
}

const routes = Object.keys(pages) as Page[];
export const navigation = Array.from(
  new Set(routes.map((id) => pages[id].group)),
  (group) => ({
    group,
    routes: routes.filter((id) => pages[id].group === group),
  }),
);
