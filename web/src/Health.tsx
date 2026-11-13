import type { Device } from "./contracts";
import { healthStale } from "./health-model";

export function Health({ device, now }: { device: Device; now: number }) {
  const report = device.health;
  if (!report)
    return (
      <section>
        <h3>设备健康</h3>
        <p>尚无健康采样。请核对客户端版本与服务运行状态。</p>
      </section>
    );
  const entries = [
    ["客户端版本", report.agent_version],
    ["镜像 / 部署版本", report.deployment_version],
    ["运行时间（秒）", report.health.uptime_seconds],
    ["CPU 使用率（%）", report.health.cpu_percent],
    ["物理内存", report.health.memory],
    ["活动显示器", report.health.displays],
    ["键盘设备数", report.health.keyboards],
    ["鼠标设备数", report.health.mice],
  ] as const;
  return (
    <section aria-label="设备健康">
      <h3>设备健康</h3>
      <p>
        {healthStale(device, now) ? "数据已过期" : "最近采样"} · 服务器接收{" "}
        {device.health_received_at
          ? new Date(device.health_received_at * 1000).toLocaleString("zh-CN")
          : "未知"}{" "}
        · 采集耗时 {report.collection_ms} ms
      </p>
      <p>
        约每 30 秒采集。输入设备状态来自 Windows
        设备枚举，显示器来自当前桌面显示配置。
      </p>
      <table>
        <thead>
          <tr>
            <th>项目</th>
            <th>值 / 状态</th>
            <th>采集时间</th>
          </tr>
        </thead>
        <tbody>
          {entries.map(([label, sample]) => (
            <tr key={label}>
              <td>{label}</td>
              <td>
                {sample.observation.status === "available"
                  ? typeof sample.observation.value === "object"
                    ? Array.isArray(sample.observation.value)
                      ? sample.observation.value
                          .map(
                            (d) =>
                              `${d.description} ${d.width} × ${d.height} @ ${d.refresh_hz} Hz`,
                          )
                          .join("、")
                      : `${(sample.observation.value.total_bytes / 2 ** 30).toFixed(1)} GiB · 使用 ${sample.observation.value.used_percent}% · 可用 ${(sample.observation.value.available_bytes / 2 ** 30).toFixed(1)} GiB`
                    : String(sample.observation.value)
                  : [
                        "not_applicable",
                        "not_configured",
                        "unsupported",
                        "missing",
                      ].includes(sample.observation.reason)
                    ? `未知 / ${sample.observation.reason}`
                    : `采集失败 / ${sample.observation.reason}`}
              </td>
              <td>
                {new Date(sample.sampled_at * 1000).toLocaleString("zh-CN")}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
