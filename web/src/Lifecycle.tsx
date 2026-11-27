import { useAccess } from "./access";
import { useEffect, useState } from "react";
import { z } from "zod";
import { api, type Mutation } from "./api";
import { type Device, type Snapshot, resultSchema } from "./contracts";
import { DetailsDialog } from "./DetailsDialog";

export const lifecycleLabels: Record<string, string> = {
  pending: "待审核",
  enabled: "已启用",
  disabled: "已禁用",
  revoked: "已撤销",
  rejected: "已拒绝",
  expired: "注册已过期",
};
export function randomSecret() {
  return Array.from(crypto.getRandomValues(new Uint8Array(32)), (value) =>
    value.toString(16).padStart(2, "0"),
  ).join("");
}
export function Material({
  material,
  onClose,
}: {
  material: string;
  onClose: () => void;
}) {
  return (
    <div className="enrollment">
      <p>注册材料有效期 24 小时。请保存到核对过的设备独立身份目录。</p>
      <textarea
        aria-label="重新授权注册材料"
        readOnly
        rows={6}
        value={material}
      />
      <div className="actions">
        <button
          onClick={() => {
            const url = URL.createObjectURL(
              new Blob([material], { type: "application/json" }),
            );
            const anchor = document.createElement("a");
            anchor.href = url;
            anchor.download = "device.local.json";
            anchor.click();
            setTimeout(() => URL.revokeObjectURL(url), 1000);
          }}
        >
          下载注册材料
        </button>
        <button onClick={onClose}>关闭材料</button>
      </div>
    </div>
  );
}
const eventLabels: Record<string, string> = {
  created: "创建设备",
  registration_requested: "提交注册申请",
  image_registration_requested: "镜像设备申请，等待人工核对",
  image_approved: "镜像凭据已审核更新",
  image_rejected: "镜像申请已拒绝",
  automatically_approved: "窗口自动批准",
  credential_rotated: "凭据切换完成",
  expired: "注册过期",
};
const actions: Record<string, { label: string; impact: string }> = {
  approve: {
    label: "批准注册",
    impact: "核对目标设备和申请关联号后启用本次申请的独立凭据。",
  },
  reject: {
    label: "拒绝注册",
    impact: "终止本次材料与申请。后续接入需要重新授权。",
  },
  disable: {
    label: "禁用设备",
    impact: "暂停后续任务领取，保留座位占用。已经开始的只读检查可以回传。",
  },
  enable: {
    label: "启用设备",
    impact: "恢复未过期任务领取。过期任务需要重新检查。",
  },
  revoke: {
    label: "撤销凭据",
    impact:
      "立即失效当前凭据，取消未完成任务并释放座位占用。历史和全局配置目标保留。",
  },
  rotate: {
    label: "更新凭据",
    impact:
      "24 小时内由新版 Windows 客户端切换凭据。切换提交后旧凭据停止普通访问。",
  },
  reauthorize: {
    label: "重新授权",
    impact:
      "核对设备身份后签发新材料。使用独立目录模拟重装，申请始终需要人工批准，接入后核对地址并显式重查。",
  },
};
export function allowedActions(device: Device): string[] {
  switch (device.lifecycle) {
    case "pending":
      return device.application_id ? ["approve", "reject"] : ["reject"];
    case "enabled":
      return ["disable", "rotate", "revoke"];
    case "disabled":
      return ["enable", "rotate", "revoke"];
    default:
      return ["reauthorize"];
  }
}

export function LifecycleControls({
  device,
  disabled,
  onMutate,
}: {
  device: Device;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const access = useAccess();
  const [confirmation, setConfirmation] = useState<Record<
    string,
    unknown
  > | null>(null);
  const [material, setMaterial] = useState("");
  const [error, setError] = useState("");
  const action = confirmation ? String(confirmation.action) : "";
  async function submit() {
    if (!confirmation) return;
    try {
      await onMutate(
        "devices/action",
        confirmation,
        `${device.name} · ${actions[action].label}已提交`,
      );
      if (action === "reauthorize")
        setMaterial(
          JSON.stringify(
            {
              server: location.origin,
              device_id: device.id,
              enrollment_token: confirmation.enrollment_token,
            },
            null,
            2,
          ),
        );
      setConfirmation(null);
      setError("");
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "操作失败，可重试相同请求",
      );
    }
  }
  return (
    <section className="lifecycle-controls">
      <h3>设备生命周期</h3>
      <p>
        客户端版本 {device.agent_version ?? "尚无心跳"} · 凭据更新需要 0.2.0
        或更新版本
      </p>
      <p>
        {lifecycleLabels[device.lifecycle]} · 状态版本{" "}
        {device.lifecycle_version} · 身份代次 {device.enrollment_generation}
      </p>
      {device.application_id && (
        <p className="mono">申请关联号 {device.application_id}</p>
      )}
      {device.lifecycle === "pending" && (
        <p>
          材料截止{" "}
          {new Date(device.enrollment_expires * 1000).toLocaleString("zh-CN")}
        </p>
      )}
      {device.rotation_id && (
        <p>
          轮换请求 {device.rotation_id} · 截止{" "}
          {new Date((device.rotation_expires ?? 0) * 1000).toLocaleString(
            "zh-CN",
          )}
        </p>
      )}
      <div className="actions">
        {allowedActions(device)
          .filter(
            (action) =>
              access.manage ||
              (access.operate && ["enable", "disable"].includes(action)),
          )
          .map((action) => (
            <button
              key={action}
              disabled={disabled || !!confirmation}
              onClick={() => {
                setError("");
                setConfirmation({
                  device_id: device.id,
                  base_version: device.lifecycle_version,
                  action,
                  ...(action === "reauthorize"
                    ? { enrollment_token: randomSecret() }
                    : {}),
                });
              }}
            >
              {actions[action].label}
            </button>
          ))}
      </div>
      {confirmation && (
        <div className="operation-preview">
          <h4>
            {actions[action].label} · {device.name}
          </h4>
          <p>
            目标 {device.id} · 基准版本 {String(confirmation.base_version)}
          </p>
          <p>{actions[action].impact}</p>
          {error && (
            <p role="alert" className="error">
              {error}。版本冲突时取消本次操作，刷新后重新核对。
            </p>
          )}
          <div className="actions">
            <button
              className="primary"
              disabled={disabled}
              onClick={() => void submit()}
            >
              {error ? "重试相同操作" : "确认提交"}
            </button>
            <button disabled={disabled} onClick={() => setConfirmation(null)}>
              取消
            </button>
          </div>
        </div>
      )}
      {material && (
        <Material material={material} onClose={() => setMaterial("")} />
      )}
    </section>
  );
}

const historySchema = z.object({
  events: z.array(
    z.object({
      sequence: z.number(),
      operation: z.string(),
      request_id: z.string(),
      version: z.number(),
      created_at: z.number(),
    }),
  ),
  jobs: z.array(
    z.object({
      sequence: z.number(),
      id: z.string(),
      config_version: z.number(),
      expires_at: z.number(),
      received_at: z.number().nullable(),
      cancelled_at: z.number().nullable(),
      cancellation_reason: z.string().nullable(),
      result: resultSchema.nullable(),
    }),
  ),
});
export function DeviceHistory({ device }: { device: Device }) {
  const [history, setHistory] = useState<z.infer<typeof historySchema> | null>(
    null,
  );
  const [error, setError] = useState("");
  const [reload, setReload] = useState(0);
  const [before, setBefore] = useState<number | null>(null);
  const [jobsBefore, setJobsBefore] = useState<number | null>(null);
  useEffect(() => {
    let active = true;
    const query = new URLSearchParams();
    if (before !== null) query.set("before", String(before));
    if (jobsBefore !== null) query.set("jobs_before", String(jobsBefore));
    api(`devices/${encodeURIComponent(device.id)}/history?${query}`)
      .then((value) => {
        const parsed = historySchema.parse(value);
        if (active) {
          setHistory(parsed);
          setError("");
        }
      })
      .catch((error) => {
        if (active)
          setError(error instanceof Error ? error.message : "历史加载失败");
      });
    return () => {
      active = false;
    };
  }, [
    device.id,
    device.lifecycle_version,
    device.received_at,
    device.desired_job,
    reload,
    before,
    jobsBefore,
  ]);
  return (
    <section>
      <h3>生命周期与任务历史</h3>
      <button
        onClick={() => {
          setBefore(null);
          setJobsBefore(null);
          setReload((value) => value + 1);
        }}
      >
        重新加载历史
      </button>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {!history && !error && <p role="status">正在读取历史…</p>}
      {history && (
        <>
          <ul>
            {history.events.map((event) => (
              <li key={event.sequence}>
                {new Date(event.created_at * 1000).toLocaleString("zh-CN")} ·{" "}
                {actions[event.operation]?.label ??
                  eventLabels[event.operation] ??
                  event.operation}{" "}
                · v{event.version}
                <small className="mono">{event.request_id}</small>
              </li>
            ))}
          </ul>
          {history.events.length === 100 && (
            <button onClick={() => setBefore(history.events.at(-1)!.sequence)}>
              更早事件
            </button>
          )}
          <h4>任务历史 · 每页 100 项</h4>
          {history.jobs.map((job) => (
            <details key={job.id}>
              <summary>
                v{job.config_version} ·{" "}
                {job.cancelled_at
                  ? "已取消"
                  : job.received_at
                    ? "已回传"
                    : "待回传 / 过期"}{" "}
                · {job.id}
              </summary>
              <p>
                {job.cancellation_reason === "device_revoked"
                  ? "设备凭据撤销，任务取消"
                  : job.cancellation_reason}
              </p>
              <pre>{JSON.stringify(job.result, null, 2)}</pre>
            </details>
          ))}
          {history.jobs.length === 100 && (
            <button
              onClick={() => setJobsBefore(history.jobs.at(-1)!.sequence)}
            >
              更早任务
            </button>
          )}
        </>
      )}
    </section>
  );
}

export function Enrollment({
  snapshot,
  disabled,
  onMutate,
}: {
  snapshot: Snapshot;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const [detailId, setDetailId] = useState<string | null>(null);
  const [filter, setFilter] = useState("pending");
  const registration = snapshot.registration_window;
  const detail = snapshot.devices.find((device) => device.id === detailId);
  const pending = snapshot.devices.filter(
    (device) => device.lifecycle === filter && !device.image_managed,
  );
  return (
    <section className="panel">
      <div className="section-heading">
        <div>
          <h2>注册窗口</h2>
          <p>
            镜像设备在窗口开启后自动申请，工作人员核对来源 IP
            和申请关联号后批准。窗口关闭时新镜像设备会继续重试。
          </p>
          <p>
            {registration.open
              ? `已开启，${new Date(registration.closes_at * 1000).toLocaleString("zh-CN")} 关闭`
              : "已关闭，新申请等待管理员审核"}
          </p>
        </div>
        <button
          disabled={disabled}
          onClick={() =>
            void onMutate(
              "registration-window",
              { base_version: registration.version, open: !registration.open },
              "注册窗口已更新",
            ).catch(() => {})
          }
        >
          {registration.open ? "关闭注册窗口" : "开启注册窗口 1 小时"}
        </button>
      </div>
      <p>
        开启后，首次使用有效逐机材料的申请自动批准。已有待审核申请及重新授权申请需要人工处理。服务重启保留窗口截止时间。
      </p>
      {snapshot.image_applications && (
        <ImageApplications
          applications={snapshot.image_applications}
          disabled={disabled}
          onMutate={onMutate}
        />
      )}
      <div className="filter-bar">
        <label>
          申请筛选
          <select
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
          >
            {["pending", "rejected", "expired"].map((value) => (
              <option key={value} value={value}>
                {lifecycleLabels[value]}
              </option>
            ))}
          </select>
        </label>
        <a href="#/devices">新增设备与注册材料</a>
      </div>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th>设备</th>
              <th>申请状态</th>
              <th>身份代次</th>
              <th>材料截止</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {pending.map((device) => (
              <tr key={device.id}>
                <td>
                  {device.name}
                  <small>{device.id}</small>
                </td>
                <td>
                  {device.lifecycle === "pending" && !device.application_id
                    ? "等待设备提交申请"
                    : lifecycleLabels[device.lifecycle]}
                </td>
                <td>{device.enrollment_generation}</td>
                <td>
                  {new Date(device.enrollment_expires * 1000).toLocaleString(
                    "zh-CN",
                  )}
                </td>
                <td>
                  <button onClick={() => setDetailId(device.id)}>
                    审核详情
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {!pending.length && <p className="empty">没有符合条件的申请</p>}
      {detail && (
        <DetailsDialog title={detail.name} onClose={() => setDetailId(null)}>
          <LifecycleControls
            key={detail.id}
            device={detail}
            disabled={disabled}
            onMutate={onMutate}
          />
          <DeviceHistory key={`history-${detail.id}`} device={detail} />
        </DetailsDialog>
      )}
    </section>
  );
}

function ImageApplications({
  applications,
  disabled,
  onMutate,
}: {
  applications: NonNullable<Snapshot["image_applications"]>;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const [selected, setSelected] = useState<
    Record<
      string,
      { application_id: string; base_version: number; review_token?: string }
    >
  >({});
  const [confirmed, setConfirmed] = useState(false);
  const targets = Object.values(selected);
  function toggle(
    application: (typeof applications)[number],
    checked: boolean,
  ) {
    setConfirmed(false);
    setSelected((previous) => {
      const next = { ...previous };
      if (checked)
        next[application.id] = {
          application_id: application.id,
          base_version: application.base_version,
          review_token: application.review_token,
        };
      else delete next[application.id];
      return next;
    });
  }
  async function submit(approve: boolean) {
    try {
      await onMutate(
        "images/action",
        { approve, targets },
        approve ? "镜像申请已批准，等待客户端重连并核对座位" : "镜像申请已拒绝",
      );
      setSelected({});
      setConfirmed(false);
    } catch {
      /* The parent displays the API failure and retains the request for retry. */
    }
  }
  return (
    <section aria-label="镜像设备审核">
      <h3>镜像设备与复原后接入</h3>
      <p>
        按当前系统机器名关联原设备。批准会更新设备凭据并取消旧任务，保留座位和历史。原设备仍在线时请先核对同名机器。比赛控制需要重新下发。
      </p>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th>选择</th>
              <th>机器名</th>
              <th>本次来源 IP</th>
              <th>原来源 IP</th>
              <th>核对提示</th>
              <th>提交时间</th>
            </tr>
          </thead>
          <tbody>
            {applications.map((application) => (
              <tr key={application.id}>
                <td>
                  <input
                    type="checkbox"
                    aria-label={`选择 ${application.machine_name} ${application.id}`}
                    checked={Boolean(selected[application.id])}
                    disabled={disabled}
                    onChange={(event) =>
                      toggle(application, event.target.checked)
                    }
                  />
                </td>
                <td>
                  {application.machine_name}
                  <small>{application.id}</small>
                </td>
                <td>{application.observed_ip ?? "来源未知"}</td>
                <td>{application.previous_ip ?? "首次接入"}</td>
                <td>
                  {{
                    "": "",
                    ready: "",
                    bootstrap_failed:
                      "客户端初始化失败，请现场重试并查看启动状态",
                    support_packages_failed: "工具运行时恢复失败，请现场重试",
                    login_template_failed:
                      "比赛账户模板导入失败，请核对账户、系统密钥与工具路径",
                    service_start_failed:
                      "客户端服务启动失败，请现场诊断并重试",
                  }[application.bootstrap_status] ||
                    (application.online
                      ? "原设备仍在线"
                      : application.competing > 1
                        ? "同名存在多个申请，每次请选择一个"
                        : "待核对")}
                </td>
                <td>
                  {new Date(application.created_at * 1000).toLocaleString(
                    "zh-CN",
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {!applications.length && <p className="empty">没有待审核的镜像申请</p>}
      <label>
        <input
          type="checkbox"
          checked={confirmed}
          disabled={disabled || !targets.length}
          onChange={(event) => setConfirmed(event.target.checked)}
        />
        已核对所选机器名与来源 IP，确认更新这些设备的凭据
      </label>
      <div className="actions">
        <button
          disabled={disabled || !targets.length || !confirmed}
          onClick={() => void submit(true)}
        >
          批准所选 {targets.length} 台
        </button>
        <button
          disabled={disabled || !targets.length}
          onClick={() => void submit(false)}
        >
          拒绝所选申请
        </button>
      </div>
    </section>
  );
}
