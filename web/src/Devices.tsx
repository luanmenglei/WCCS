import { useState, type FormEvent } from "react";
import { checkLabel, reasons, type Device, type Snapshot } from "./contracts";

type Props = {
  snapshot: Snapshot;
  disabled: boolean;
  onCreate: (body: Record<string, unknown>) => Promise<void>;
  onRecheck: (id: string) => Promise<void>;
};
function time(value: number | null) {
  return value === null
    ? "尚无记录"
    : new Date(value * 1000).toLocaleString("zh-CN", { hour12: false });
}
function status(device: Device) {
  if (!device.registered) return "待注册";
  if (device.expired) return "任务已过期";
  if (!device.result) return device.desired_job ? "等待回传" : "尚未下发";
  if (device.stale) return "结果已过期";
  return device.result.tools.some((item) => item.check.status === "unavailable")
    ? "有检查异常"
    : "检查完成";
}

export function Devices({ snapshot, disabled, onCreate, onRecheck }: Props) {
  const [creating, setCreating] = useState(false);
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
          <h2 id="devices-title">设备与检查</h2>
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
      {snapshot.devices.length === 0 ? (
        <p className="empty">
          新增合成设备，启动 Windows 检查入口后即可选择下发目标。
        </p>
      ) : (
        <div className="device-list">
          {snapshot.devices.map((device) => (
            <article className="device" key={device.id}>
              <div className="device-summary">
                <div>
                  <strong>{device.name}</strong>
                  <small>{device.id}</small>
                </div>
                <span
                  className={device.online ? "status online-text" : "status"}
                >
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
                    <b>
                      {device.result ? `v${device.result.config_version}` : "—"}
                    </b>
                  </span>
                </div>
                <span
                  className={
                    device.stale || device.expired ? "status stale" : "status"
                  }
                >
                  {status(device)}
                </span>
                <button
                  disabled={disabled || !device.desired_job}
                  onClick={() => {
                    void onRecheck(device.id).catch(() => {});
                  }}
                >
                  重新检查
                </button>
              </div>
              <p className="device-time">
                最后心跳 {time(device.last_seen)} · 结果回传{" "}
                {time(device.received_at)}
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
                                ? check.value.file_version.status ===
                                  "available"
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
          ))}
        </div>
      )}
    </section>
  );
}
