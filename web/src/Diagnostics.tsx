import type { Device } from "./contracts";
import {
  diagnosticOutcomes,
  diagnosticRecovery,
  diagnosticSteps,
} from "./diagnostics-model";

function time(value: number | null) {
  return value === null
    ? "尚无记录"
    : new Date(value * 1000).toLocaleString("zh-CN", { hour12: false });
}

export function Diagnostics({ device }: { device: Device }) {
  const report = device.diagnostics;
  return (
    <section aria-label="本机接入诊断">
      <h3>本机接入诊断</h3>
      <a href={`#/maintenance?device_id=${encodeURIComponent(device.id)}`}>
        显示此设备识别画面
      </a>
      {report ? (
        <>
          <p>服务器 {report.server_origin ?? "地址待配置"}</p>
          <p>
            客户端 {report.agent_version} · 本机配置 v
            {report.configuration_revision} ·{" "}
            {report.evidence_context === "service" ? "服务诊断" : "开发机测试"}
          </p>
          <p>
            采集 {time(report.collected_at)} · 服务器接收{" "}
            {time(device.diagnostics_received_at)}
            {device.online ? "" : " · 设备离线，数据已过期"}
          </p>
          {report.seat_claim && (
            <p>
              工作人员核对信息为机房 {report.seat_claim.room_id}，座位{" "}
              {report.seat_claim.seat_number}，网卡{" "}
              {report.seat_claim.interface_index} /{" "}
              {report.seat_claim.mac_address}。请与上方服务端 IP
              匹配及冲突状态共同核对。
            </p>
          )}
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>检查环节</th>
                  <th>结果</th>
                  <th>最近成功</th>
                  <th>恢复操作 / 回执</th>
                </tr>
              </thead>
              <tbody>
                {report.checks.map((check) => (
                  <tr key={check.step}>
                    <td>{diagnosticSteps[check.step]}</td>
                    <td>{diagnosticOutcomes[check.outcome]}</td>
                    <td>{time(check.last_success_at)}</td>
                    <td>{diagnosticRecovery[check.code]}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <details>
            <summary>关联编号与配置摘要</summary>
            <p className="mono">{report.request_id}</p>
            <p className="mono">{report.configuration_digest}</p>
          </details>
        </>
      ) : (
        <p>
          尚未收到诊断报告。在目标机器运行 wccs-agent --setup
          身份文件，保存配置后执行分步骤诊断。
        </p>
      )}
    </section>
  );
}
