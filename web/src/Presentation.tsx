import { useEffect, useState } from "react";
import { z } from "zod";
import { api } from "./api";
import type { Mutation } from "./SeatChecks";

const logoSchema = z.object({
  school: z.string(),
  source: z.string(),
  sha256: z.string(),
  bmp_hex: z.string(),
});
const settingsSchema = z.object({
  starts_at_ms: z.number().nullable(),
  domjudge_api: z.string().nullable(),
  domjudge_contest: z.string().nullable(),
  logos: z.array(logoSchema),
});
const receiptSchema = z.object({
  device_id: z.string(),
  received_at: z.number(),
  report: z.object({
    round_trip_ms: z.number(),
    clock_offset_ms: z.number(),
    wccs: z.object({
      state: z.string(),
      source: z.string(),
      checked_at_ms: z.number(),
      detail: z.string(),
    }),
    oj: z.object({
      state: z.string(),
      source: z.string(),
      checked_at_ms: z.number(),
      detail: z.string(),
    }),
    data: z.object({
      configuration_version: z.number(),
      configuration_digest: z.string(),
      identity: z.object({
        seat_label: z.string(),
        contest_version: z.number(),
        room_version: z.number(),
      }),
    }),
  }),
});
const responseSchema = z.object({
  version: z.number(),
  settings: settingsSchema,
  receipts: z.array(receiptSchema),
});
const states: Record<string, string> = {
  online: "已连接",
  offline: "连接中断",
  unknown: "未知",
};
const details: Record<string, string> = {
  receipt_verified: "实际 API 回执已核对",
  not_configured: "未配置",
  http_unavailable: "请求失败",
  authentication_required: "API 要求认证",
  response_invalid: "响应格式不符合比赛 API",
};

export function Presentation({
  contestId,
  disabled,
  onMutate,
}: {
  contestId: string;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const [version, setVersion] = useState(0);
  const [settings, setSettings] = useState<z.infer<typeof settingsSchema>>({
    starts_at_ms: null,
    domjudge_api: null,
    domjudge_contest: null,
    logos: [],
  });
  const [receipts, setReceipts] = useState<z.infer<typeof receiptSchema>[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [school, setSchool] = useState("");
  async function reload() {
    const value = responseSchema.parse(
      await api(`presentation?contest_id=${encodeURIComponent(contestId)}`),
    );
    setVersion(value.version);
    setSettings(value.settings);
    setReceipts(value.receipts);
    setLoaded(true);
  }
  useEffect(() => {
    let active = true;
    if (!contestId) return;
    void api(`presentation?contest_id=${encodeURIComponent(contestId)}`)
      .then(responseSchema.parse)
      .then((value) => {
        if (active) {
          setVersion(value.version);
          setSettings(value.settings);
          setReceipts(value.receipts);
          setLoaded(true);
        }
      })
      .catch((error) => {
        if (active)
          setError(error instanceof Error ? error.message : "加载失败");
      });
    return () => {
      active = false;
    };
  }, [contestId]);
  if (!contestId) return <p className="panel">选择比赛后配置等待展示。</p>;
  async function upload(file: File) {
    setError("");
    try {
      if (!school.trim() || file.size > 28_000)
        throw new Error("先填写名单中的学校名称，BMP 文件最大 28 KB。");
      const bytes = await file.arrayBuffer();
      const sha256 = Array.from(
        new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
        (v) => v.toString(16).padStart(2, "0"),
      ).join("");
      const bmp_hex = Array.from(new Uint8Array(bytes), (v) =>
        v.toString(16).padStart(2, "0"),
      ).join("");
      setSettings((previous) => ({
        ...previous,
        logos: [
          ...previous.logos.filter((l) => l.school !== school.trim()),
          {
            school: school.trim(),
            source: `工作人员导入 ${file.name}`,
            sha256,
            bmp_hex,
          },
        ],
      }));
    } catch (error) {
      setError(error instanceof Error ? error.message : "校徽读取失败");
    }
  }
  return (
    <>
      <section className="panel operations-panel">
        <h2>等待展示配置 · v{version}</h2>
        <fieldset disabled={disabled || busy || !loaded}>
          <label>
            开始时间（本地时区）
            <input
              type="datetime-local"
              value={
                settings.starts_at_ms === null
                  ? ""
                  : new Date(
                      settings.starts_at_ms -
                        new Date(settings.starts_at_ms).getTimezoneOffset() *
                          60000,
                    )
                      .toISOString()
                      .slice(0, 16)
              }
              onChange={(e) =>
                setSettings({
                  ...settings,
                  starts_at_ms: e.target.value
                    ? new Date(e.target.value).getTime()
                    : null,
                })
              }
            />
          </label>
          <label>
            DOMjudge API 根地址
            <input
              value={settings.domjudge_api ?? ""}
              placeholder="https://judge.example/api/v4"
              onChange={(e) =>
                setSettings({
                  ...settings,
                  domjudge_api: e.target.value || null,
                })
              }
            />
          </label>
          <label>
            DOMjudge 比赛编号
            <input
              value={settings.domjudge_contest ?? ""}
              onChange={(e) =>
                setSettings({
                  ...settings,
                  domjudge_contest: e.target.value || null,
                })
              }
            />
          </label>
          <p>
            客户端请求该比赛的公开
            API，核对比赛编号与响应格式。要求认证时显示对应失败环节。开始时间仅用于展示。
          </p>
          <label>
            学校名称
            <input value={school} onChange={(e) => setSchool(e.target.value)} />
          </label>
          <label>
            学校校徽（96 × 96 以内、24 位无压缩 BMP）
            <input
              type="file"
              accept=".bmp"
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file) void upload(file);
                e.target.value = "";
              }}
            />
          </label>
          <p>
            校徽随配置从 WCCS 内网交付并保存在客户端展示缓存，保存时验证格式与
            SHA-256。
          </p>
          {settings.logos.map((logo) => (
            <p key={logo.school}>
              {logo.school} · {logo.source} <code>{logo.sha256}</code>{" "}
              <button
                type="button"
                onClick={() =>
                  setSettings({
                    ...settings,
                    logos: settings.logos.filter((l) => l !== logo),
                  })
                }
              >
                移除
              </button>
            </p>
          ))}
          <button
            className="primary"
            onClick={() => {
              setBusy(true);
              setError("");
              void onMutate(
                "presentation",
                { contest_id: contestId, base_version: version, settings },
                "展示配置已保存，等待客户端刷新及校验回执。",
              )
                .then(reload)
                .catch((error) =>
                  setError(error instanceof Error ? error.message : "保存失败"),
                )
                .finally(() => setBusy(false));
            }}
          >
            保存展示配置
          </button>
        </fieldset>
        <button
          disabled={busy}
          onClick={() =>
            void reload().catch((error) => setError(String(error)))
          }
        >
          重新加载配置与回执
        </button>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
      </section>
      <section className="panel operations-panel">
        <h2>设备展示诊断</h2>
        <p>
          等待屏按 F1
          显示同一身份、配置摘要、时间样本和资源来源。离线后控制按既有租约退出。
        </p>
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>设备 / 座位</th>
                <th>配置与身份版本</th>
                <th>WCCS</th>
                <th>OJ</th>
                <th>时间样本</th>
              </tr>
            </thead>
            <tbody>
              {receipts.map((item) => (
                <tr key={item.device_id}>
                  <td>
                    {item.device_id} · {item.report.data.identity.seat_label}
                    {Date.now() / 1000 - item.received_at > 60
                      ? " · 回执过期"
                      : ""}
                  </td>
                  <td>
                    展示 v{item.report.data.configuration_version} · 比赛 v
                    {item.report.data.identity.contest_version} · 座位 v
                    {item.report.data.identity.room_version}
                    <small className="mono">
                      {item.report.data.configuration_digest}
                    </small>
                  </td>
                  {[item.report.wccs, item.report.oj].map(
                    (connection, index) => (
                      <td key={index}>
                        {states[connection.state] ?? connection.state}
                        <small>
                          {details[connection.detail] ?? connection.detail}
                        </small>
                        <small>
                          {connection.source} ·{" "}
                          {new Date(connection.checked_at_ms).toLocaleString(
                            "zh-CN",
                          )}
                        </small>
                      </td>
                    ),
                  )}
                  <td>
                    RTT {item.report.round_trip_ms} ms · 偏差{" "}
                    {item.report.clock_offset_ms} ms
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}
