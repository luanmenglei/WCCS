import { useState, type FormEvent } from "react";
import {
  LifecycleControls,
  DeviceHistory,
  lifecycleLabels,
  type Mutate,
} from "./Lifecycle";
import { DetailsDialog } from "./DetailsDialog";
import { checkStatus, matchesSearch, seatRows } from "./fleet";
import { checkLabel, reasons, type Device, type Snapshot } from "./contracts";

type Props = {
  snapshot: Snapshot;
  onMutate: Mutate;
  disabled: boolean;
  onCreate: (body: Record<string, unknown>) => Promise<void>;
  onRecheck: (id: string) => Promise<void>;
};
function time(value: number | null) {
  return value === null
    ? "尚无记录"
    : new Date(value * 1000).toLocaleString("zh-CN", { hour12: false });
}

export function Devices({
  snapshot,
  disabled,
  onCreate,
  onRecheck,
  onMutate,
}: Props) {
  const [creating, setCreating] = useState(false);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [descending, setDescending] = useState(false);
  const [detailId, setDetailId] = useState<string | null>(null);
  const seats = seatRows(snapshot);
  const addresses = new Map(
    snapshot.seating.devices.map((device) => [device.id, device]),
  );
  const bindings = new Map(
    snapshot.devices.map((device) => [
      device.id,
      seats.filter(({ seat }) => seat.devices.includes(device.id)),
    ]),
  );
  const rows = snapshot.devices
    .filter((device) => {
      const matched = bindings.get(device.id) ?? [];
      const address = addresses.get(device.id);
      const stateMatches =
        filter === "all" ||
        filter === device.lifecycle ||
        (filter === "online" && device.online) ||
        (filter === "offline" && !device.online && device.registered) ||
        (filter === "unregistered" && device.lifecycle === "pending") ||
        (filter === "unbound" && !matched.length) ||
        (filter === "conflict" &&
          matched.some(({ seat }) => seat.status === "conflict"));
      return (
        stateMatches &&
        matchesSearch(
          [
            device.id,
            device.name,
            address?.observed_ip,
            ...matched.flatMap(({ room, seat }) => [room.name, seat.number]),
          ],
          search,
        )
      );
    })
    .sort(
      (a, b) =>
        (descending ? -1 : 1) *
        a.name.localeCompare(b.name, "zh-CN", { numeric: true }),
    );
  const detail = snapshot.devices.find((device) => device.id === detailId);
  const [material, setMaterial] = useState("");
  const [localError, setLocalError] = useState("");
  const [enrollmentToken, setEnrollmentToken] = useState(() =>
    Array.from(crypto.getRandomValues(new Uint8Array(32)), (value) =>
      value.toString(16).padStart(2, "0"),
    ).join(""),
  );
  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const id = String(form.get("id")).trim();
    try {
      await onCreate({
        id,
        name: String(form.get("name")).trim(),
        enrollment_token: enrollmentToken,
      });
      setMaterial(
        JSON.stringify(
          {
            server: location.origin,
            device_id: id,
            enrollment_token: enrollmentToken,
          },
          null,
          2,
        ),
      );
      setEnrollmentToken(
        Array.from(crypto.getRandomValues(new Uint8Array(32)), (value) =>
          value.toString(16).padStart(2, "0"),
        ).join(""),
      );
      setCreating(false);
      setLocalError("");
    } catch (error) {
      setLocalError(error instanceof Error ? error.message : "设备创建失败");
    }
  }
  function download() {
    const url = URL.createObjectURL(
      new Blob([material], { type: "application/json" }),
    );
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "device.local.json";
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  return (
    <section
      id="devices"
      aria-labelledby="devices-title"
      className="panel devices-panel"
    >
      <div className="section-heading">
        <div>
          <h2 id="devices-title">设备清单</h2>
          <p>
            {snapshot.devices.filter((device) => device.online).length}{" "}
            台在线，共 {snapshot.devices.length} 台 · 每 3 秒刷新
          </p>
        </div>
        <button disabled={disabled} onClick={() => setCreating(!creating)}>
          新增设备
        </button>
      </div>
      {creating && (
        <form className="device-form" onSubmit={(event) => void create(event)}>
          <label>
            设备 ID
            <input name="id" required pattern="[A-Za-z0-9_-]+" maxLength={80} />
          </label>
          <label>
            设备名称
            <input name="name" required maxLength={100} />
          </label>
          <button className="primary" disabled={disabled}>
            生成注册材料
          </button>
        </form>
      )}
      {localError && (
        <p role="alert" className="error">
          {localError}
        </p>
      )}
      {material && (
        <div className="enrollment">
          <p>
            注册材料有效期 24 小时。保存到目标 Windows
            设备的受保护目录，再启动检查入口。
          </p>
          <textarea
            aria-label="设备注册材料"
            readOnly
            rows={6}
            value={material}
          />
          <div className="actions">
            <button onClick={download}>下载注册材料</button>
            <button onClick={() => setMaterial("")}>关闭材料</button>
          </div>
        </div>
      )}
      <div className="filter-bar device-filters">
        <label className="search-field">
          搜索设备
          <input
            type="search"
            placeholder="设备、机房、座位号或 IP"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
        <label>
          设备筛选
          <select
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
          >
            <option value="all">全部设备</option>
            {Object.entries(lifecycleLabels).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
            <option value="online">在线</option>
            <option value="offline">离线</option>
            <option value="unregistered">待注册</option>
            <option value="unbound">未分配</option>
            <option value="conflict">绑定冲突</option>
          </select>
        </label>
        <span className="result-count">
          {rows.length} / {snapshot.devices.length} 台设备
        </span>
      </div>
      <div className="table-scroll fleet-table">
        <table>
          <thead>
            <tr>
              <th>
                <button
                  className="table-sort"
                  onClick={() => setDescending(!descending)}
                >
                  设备名称 {descending ? "↓" : "↑"}
                </button>
              </th>
              <th>生命周期</th>
              <th>连接</th>
              <th>机房 / 座位</th>
              <th>观察 IP</th>
              <th>期望 / 实际</th>
              <th>工具检查</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((device) => (
              <tr key={device.id}>
                <td>
                  <strong>{device.name}</strong>
                  <small className="mono">{device.id}</small>
                </td>
                <td>{lifecycleLabels[device.lifecycle]}</td>
                <td>
                  <span className={device.online ? "online-text" : "status"}>
                    {device.online
                      ? "在线"
                      : device.registered
                        ? "离线"
                        : "认证停用"}
                  </span>
                </td>
                <td>
                  {bindings
                    .get(device.id)
                    ?.map(
                      ({ room, seat }) =>
                        `${room.name} / ${seat.number}${seat.status === "conflict" ? " · 冲突" : ""}`,
                    )
                    .join("、") || "未分配"}
                </td>
                <td className="mono">
                  {addresses.get(device.id)?.observed_ip ?? "未知"}
                </td>
                <td>
                  期望{" "}
                  {device.desired_version === null
                    ? "—"
                    : `v${device.desired_version}`}
                  <small>
                    实际{" "}
                    {device.result ? `v${device.result.config_version}` : "—"}
                  </small>
                </td>
                <td>{checkStatus(device)}</td>
                <td>
                  <button onClick={() => setDetailId(device.id)}>
                    查看详情
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!rows.length && (
          <p className="empty">
            {snapshot.devices.length
              ? "没有符合条件的设备，请调整筛选。"
              : "尚无设备。点击新增设备生成注册材料。"}
          </p>
        )}
      </div>
      {detail && (
        <DetailsDialog title={detail.name} onClose={() => setDetailId(null)}>
          <LifecycleControls
            key={detail.id}
            device={detail}
            disabled={disabled}
            onMutate={onMutate}
          />
          <DeviceDetails
            recheckAllowed={snapshot.targets.includes(detail.id)}
            device={detail}
            disabled={disabled}
            onRecheck={onRecheck}
          />
          <DeviceHistory key={`history-${detail.id}`} device={detail} />
        </DetailsDialog>
      )}
    </section>
  );
}

export function DeviceDetails({
  device,
  recheckAllowed = !!device.desired_job,
  disabled,
  onRecheck,
}: {
  device: Device;
  recheckAllowed?: boolean;
  disabled: boolean;
  onRecheck: (id: string) => Promise<void>;
}) {
  return (
    <article className="device">
      <div className="device-summary">
        <div>
          <strong>{device.name}</strong>
          <small>{device.id}</small>
        </div>
        <span className={device.online ? "status online-text" : "status"}>
          {device.online ? "在线" : "离线"}
        </span>
        <div className="version-pair">
          <span>
            期望{" "}
            <b>
              {device.desired_version === null
                ? "—"
                : `v${device.desired_version}`}
            </b>
          </span>
          <span>
            实际{" "}
            <b>{device.result ? `v${device.result.config_version}` : "—"}</b>
          </span>
        </div>
        <span
          className={device.stale || device.expired ? "status stale" : "status"}
        >
          {checkStatus(device)}
        </span>
        <button
          disabled={
            disabled || !recheckAllowed || device.lifecycle !== "enabled"
          }
          onClick={() => {
            void onRecheck(device.id).catch(() => {});
          }}
        >
          重新检查
        </button>
      </div>
      <p className="device-time">
        最后心跳 {time(device.last_seen)} · 结果回传 {time(device.received_at)}
      </p>
      {device.result && (
        <details open>
          <summary>
            逐项检查结果 · {device.result.tools.length} 项
            {device.stale ? " · 旧任务结果" : ""}
          </summary>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>工具</th>
                  <th>检查路径</th>
                  <th>结果 / 失败原因</th>
                  <th>文件版本</th>
                </tr>
              </thead>
              <tbody>
                {device.result.tools.map(({ tool, check }) => (
                  <tr key={tool.id}>
                    <td>
                      {tool.name}
                      <small>
                        {tool.kind === "script" ? "脚本" : "可执行文件"}
                      </small>
                    </td>
                    <td className="mono">{tool.path ?? "未配置"}</td>
                    <td
                      className={
                        check.status === "available"
                          ? "online-text"
                          : "failure-text"
                      }
                    >
                      {checkLabel(check)}
                    </td>
                    <td>
                      {check.status === "available"
                        ? check.value.file_version.status === "available"
                          ? check.value.file_version.value
                          : reasons[check.value.file_version.reason]
                        : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>
      )}
    </article>
  );
}
