import { useState } from "react";
import { api } from "./api";
import type { Snapshot } from "./contracts";
import { targetPreviewSchema } from "./seating";
import type { z } from "zod";

export type Mutation = (
  path: string,
  body: Record<string, unknown>,
  message: string,
) => Promise<void>;
const labels = {
  ready: "可检查",
  unregistered: "未注册 / 未匹配，跳过",
  offline: "离线，跳过",
  conflict: "冲突，跳过",
  outside_configuration: "配置范围外，跳过",
};

export function SeatChecks({
  snapshot,
  selected,
  csrf,
  disabled,
  onMutate,
}: {
  snapshot: Snapshot;
  selected: string[];
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const [preview, setPreview] = useState<z.infer<
    typeof targetPreviewSchema
  > | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [reviewed, setReviewed] = useState("");
  const selection = JSON.stringify([snapshot.version, selected]);
  const current = reviewed === selection ? preview : null;
  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (error) {
      setError(error instanceof Error ? error.message : "操作失败，请重试");
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="check-submission">
      <button
        className="primary"
        disabled={disabled || busy || !selected.length}
        onClick={() =>
          void run(async () => {
            const result = targetPreviewSchema.parse(
              await api(
                "seating/check-preview",
                {
                  request_id: crypto.randomUUID(),
                  base_version: snapshot.version,
                  seat_ids: selected,
                },
                csrf,
              ),
            );
            setPreview(result);
            setReviewed(selection);
          })
        }
      >
        {busy ? "正在处理…" : `预览检查目标（${selected.length}）`}
      </button>
      {error && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}
      {current && (
        <section className="check-review" aria-label="实际设备目标">
          <h3>实际设备目标 · 配置 v{snapshot.version}</h3>
          <p>
            {
              current.targets.filter((target) => target.status === "ready")
                .length
            }{" "}
            台可检查，
            {
              current.targets.filter((target) => target.status !== "ready")
                .length
            }{" "}
            个座位跳过。
          </p>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>座位号</th>
                  <th>实际设备</th>
                  <th>处理结果</th>
                </tr>
              </thead>
              <tbody>
                {current.targets.map((target) => (
                  <tr key={target.seat_id}>
                    <td>{target.number}</td>
                    <td className="mono">
                      {target.device_ids.join("、") || "无设备"}
                    </td>
                    <td>{labels[target.status]}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="actions">
            <button
              disabled={
                disabled ||
                busy ||
                !current.targets.some((target) => target.status === "ready")
              }
              className="primary"
              onClick={() =>
                void run(async () => {
                  await onMutate(
                    "seating/check",
                    {
                      base_version: snapshot.version,
                      seat_ids: selected,
                      preview_token: current.preview_token,
                    },
                    "检查任务已提交，逐项结果将在设备上线回传后更新。",
                  );
                  setPreview(null);
                })
              }
            >
              确认发起工具检查
            </button>
            <button disabled={busy} onClick={() => setPreview(null)}>
              关闭预览
            </button>
          </div>
        </section>
      )}
    </div>
  );
}
