import { useEffect, useRef, useState } from "react";
import { z } from "zod";
import { api, type Mutation } from "./api";
import { useAccess } from "./access";
import type { Snapshot } from "./contracts";

const manifestSchema = z.object({
  id: z.string(),
  name: z.string(),
  size: z.number(),
  hashes: z.array(z.string()),
});
const stateSchema = z.object({
  artifacts: z.array(
    z.object({
      manifest: manifestSchema,
      ready: z.boolean(),
      uploaded_chunks: z.number(),
    }),
  ),
  jobs: z.array(
    z.object({
      id: z.string(),
      device_id: z.string(),
      device_name: z.string(),
      artifact_id: z.string(),
      stage: z.string(),
      verified_chunks: z.number(),
      detail: z.string(),
      directory: z.string(),
      adapter: z.string(),
      publish_at: z.number(),
      updated_at: z.number(),
    }),
  ),
});
const previewSchema = z.object({
  preview_token: z.string(),
  directory: z.string(),
  targets: z.array(
    z.object({
      device_id: z.string(),
      name: z.string(),
      adapter: z.string().nullable(),
      reason: z.string().nullable(),
    }),
  ),
});
const labels: Record<string, string> = {
  queued: "等待客户端下载",
  downloading: "下载中",
  ready: "校验完成，等待发布",
  publishing: "客户端已领取发布，等待实际结果",
  publication_unconfirmed: "发布期间授权已变化，需核对本机文件",
  published: "已发布",
  failed: "失败",
  cancelled: "已取消",
  device_not_ready: "设备待核对或已停用",
  client_offline_or_unsupported: "分发客户端离线或尚未升级",
  transfer_active: "已有同一文件的分发任务",
  ordinary_account_unavailable: "普通比赛账户桌面尚未准备",
  chunks_verified: "已校验分块",
  awaiting_publication: "等待发布时间",
  file_published: "文件已发布",
  transfer_failed: "本地文件操作失败，请核对空间与权限",
  target_changed: "发布目标已变化",
  target_reparse_point: "目录或文件存在重解析点",
  file_conflict: "已存在同名且内容不同的文件",
  cache_corrupt: "缓存校验失败，请补发",
  cancelled_by_operator: "操作员已取消",
  authorization_changed: "设备身份或操作员授权已变化",
};
const chunkSize = 256 * 1024;
async function sha256(bytes: ArrayBuffer) {
  return Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
    (byte) => byte.toString(16).padStart(2, "0"),
  ).join("");
}

export function Distribution({
  snapshot,
  csrf,
  disabled,
  onMutate,
}: {
  snapshot: Snapshot;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const access = useAccess();
  const [state, setState] = useState<z.infer<typeof stateSchema> | null>(null);
  const [profileVersion, setProfileVersion] = useState(0);
  const [artifactId, setArtifactId] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [room, setRoom] = useState("");
  const [publishAt, setPublishAt] = useState("");
  const [limit, setLimit] = useState(1024);
  const [preview, setPreview] = useState<z.infer<typeof previewSchema> | null>(
    null,
  );
  const [busy, setBusy] = useState(false);
  const [uploadStatus, setUploadStatus] = useState("");
  const [error, setError] = useState("");
  const active = useRef(true);
  const uploadCancelled = useRef(false);
  useEffect(() => {
    active.current = true;
    let pending = false;
    async function refresh() {
      if (pending) return;
      pending = true;
      try {
        const [value, profile] = await Promise.all([
          api("distribution"),
          api("preparation"),
        ]);
        if (active.current) {
          setState(stateSchema.parse(value));
          setProfileVersion(
            z.object({ version: z.number() }).parse(profile).version,
          );
        }
      } catch (error) {
        if (active.current)
          setError(error instanceof Error ? error.message : "读取失败");
      } finally {
        pending = false;
      }
    }
    void refresh();
    const timer = setInterval(() => void refresh(), 3000);
    return () => {
      active.current = false;
      uploadCancelled.current = true;
      clearInterval(timer);
    };
  }, []);
  async function upload(file: File) {
    setBusy(true);
    setError("");
    uploadCancelled.current = false;
    try {
      if (file.size === 0 || file.size > 512 * 1024 * 1024)
        throw new Error("请选择 1 字节到 512 MiB 的文件。");
      const hashes: string[] = [];
      for (let offset = 0; offset < file.size; offset += chunkSize) {
        if (!active.current || uploadCancelled.current)
          throw new Error("上传已暂停，再次选择同一文件可续传。");
        hashes.push(
          await sha256(
            await file.slice(offset, offset + chunkSize).arrayBuffer(),
          ),
        );
        setUploadStatus(
          `校验文件 ${Math.min(offset + chunkSize, file.size)} / ${file.size} 字节`,
        );
      }
      const id = await sha256(
        new TextEncoder().encode(
          JSON.stringify({ name: file.name, size: file.size, hashes }),
        ).buffer,
      );
      const manifest = { id, name: file.name, size: file.size, hashes };
      const created = z
        .object({ uploaded: z.array(z.number()) })
        .parse(await api("distribution/artifacts", manifest, csrf));
      const uploaded = new Set(created.uploaded);
      for (let index = 0; index < hashes.length; index++) {
        if (!active.current || uploadCancelled.current)
          throw new Error("上传已暂停，再次选择同一文件可续传。");
        if (!uploaded.has(index))
          await api(
            `distribution/artifacts/${id}/${index}`,
            file.slice(index * chunkSize, (index + 1) * chunkSize),
            csrf,
          );
        setUploadStatus(`已上传 ${index + 1} / ${hashes.length} 块`);
      }
      setArtifactId(id);
      setPreview(null);
      setUploadStatus("文件上传和摘要校验完成，可以选择设备分发。");
      setState(stateSchema.parse(await api("distribution")));
    } catch (error) {
      if (active.current)
        setError(
          error instanceof Error
            ? error.message
            : "上传失败，再次选择同一文件可续传",
        );
    } finally {
      if (active.current) setBusy(false);
    }
  }
  const roomDevices = new Set(
    snapshot.seating.rooms
      .filter((item) => !room || item.id === room)
      .flatMap((item) => item.seats.flatMap((seat) => seat.devices)),
  );
  const devices = snapshot.devices.filter((device) =>
    roomDevices.has(device.id),
  );
  const input = {
    artifact_id: artifactId,
    profile_version: profileVersion,
    device_ids: selected,
    publish_at: publishAt
      ? Math.floor(new Date(publishAt).getTime() / 1000)
      : 0,
    bytes_per_second: limit * 1024,
  };
  return (
    <section className="panel operations-panel">
      <h2>资料上传与分发</h2>
      <p>
        文件发布到比赛工作目录的 WCCS-resources
        文件夹，每个版本单独保存。断线后继续下载已授权任务，发布时间由服务器确认。
      </p>
      <label>
        上传文件（最多 512 MiB）
        <input
          type="file"
          disabled={disabled || busy || !access.manage}
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) void upload(file);
            event.target.value = "";
          }}
        />
      </label>
      {uploadStatus && <p role="status">{uploadStatus}</p>}
      {busy && (
        <button
          onClick={() => {
            uploadCancelled.current = true;
          }}
        >
          暂停上传
        </button>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <fieldset disabled={disabled || busy || !access.operate}>
        <label>
          文件版本
          <select
            value={artifactId}
            onChange={(event) => {
              setArtifactId(event.target.value);
              setPreview(null);
            }}
          >
            <option value="">请选择文件</option>
            {state?.artifacts.map((item) => (
              <option
                key={item.manifest.id}
                value={item.manifest.id}
                disabled={!item.ready}
              >
                {item.manifest.name} · {(item.manifest.size / 1024).toFixed(1)}{" "}
                KiB · {item.manifest.id.slice(0, 8)}
                {!item.ready ? " · 尚未传完" : ""}
              </option>
            ))}
          </select>
        </label>
        <label>
          发布时间（留空表示校验后立即发布）
          <input
            type="datetime-local"
            value={publishAt}
            onChange={(event) => {
              setPublishAt(event.target.value);
              setPreview(null);
            }}
          />
        </label>
        <label>
          每台下载上限（KiB/s）
          <input
            type="number"
            min={64}
            max={16384}
            value={limit}
            onChange={(event) => {
              setLimit(Number(event.target.value));
              setPreview(null);
            }}
          />
        </label>
        <label>
          机房
          <select
            value={room}
            onChange={(event) => setRoom(event.target.value)}
          >
            <option value="">全部授权机房</option>
            {snapshot.seating.rooms.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name}
              </option>
            ))}
          </select>
        </label>
        <button
          onClick={() => {
            setSelected(devices.map((device) => device.id));
            setPreview(null);
          }}
        >
          选择当前机房设备
        </button>
        <button
          onClick={() => {
            setSelected([]);
            setPreview(null);
          }}
        >
          清除选择
        </button>
        <table>
          <thead>
            <tr>
              <th>选择</th>
              <th>设备</th>
              <th>连接</th>
            </tr>
          </thead>
          <tbody>
            {devices.map((device) => (
              <tr key={device.id}>
                <td>
                  <input
                    type="checkbox"
                    aria-label={`分发到 ${device.name}`}
                    checked={selected.includes(device.id)}
                    onChange={(event) => {
                      setSelected(
                        event.target.checked
                          ? [...selected, device.id]
                          : selected.filter((id) => id !== device.id),
                      );
                      setPreview(null);
                    }}
                  />
                </td>
                <td>{device.name}</td>
                <td>{device.online ? "在线" : "离线"}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <button
          disabled={!artifactId || !profileVersion || selected.length === 0}
          onClick={() => {
            setBusy(true);
            setError("");
            setPreview(null);
            void api(
              "distribution/preview",
              { ...input, request_id: crypto.randomUUID() },
              csrf,
            )
              .then(previewSchema.parse)
              .then(setPreview)
              .catch((error) =>
                setError(error instanceof Error ? error.message : "预览失败"),
              )
              .finally(() => setBusy(false));
          }}
        >
          预览分发目标
        </button>
        {preview && (
          <>
            <p>发布目录 {preview.directory}\WCCS-resources</p>
            <table>
              <thead>
                <tr>
                  <th>设备</th>
                  <th>执行环境</th>
                  <th>结果</th>
                </tr>
              </thead>
              <tbody>
                {preview.targets.map((target) => (
                  <tr key={target.device_id}>
                    <td>{target.name}</td>
                    <td>
                      {target.adapter === "simulation"
                        ? "临时目录模拟"
                        : "Windows 服务"}
                    </td>
                    <td>
                      {target.reason
                        ? (labels[target.reason] ?? target.reason)
                        : "可分发"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <button
              disabled={preview.targets.every(
                (target) => target.reason !== null,
              )}
              onClick={() => {
                setBusy(true);
                setError("");
                void onMutate(
                  "distribution/commit",
                  { ...input, preview_token: preview.preview_token },
                  "分发任务已创建，请查看逐机进度。",
                )
                  .then(() => setPreview(null))
                  .catch((error) =>
                    setError(
                      error instanceof Error ? error.message : "分发失败",
                    ),
                  )
                  .finally(() => setBusy(false));
              }}
            >
              确认分发可执行目标
            </button>
          </>
        )}
      </fieldset>
      <h3>逐机进度</h3>
      <table>
        <thead>
          <tr>
            <th>设备与文件</th>
            <th>状态</th>
            <th>已校验分块</th>
            <th>结果</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          {state?.jobs.map((job) => {
            const artifact = state.artifacts.find(
              (item) => item.manifest.id === job.artifact_id,
            );
            return (
              <tr key={job.id}>
                <td>
                  {job.device_name}
                  <br />
                  {artifact?.manifest.name ?? job.artifact_id}
                  {job.adapter === "simulation" ? " · 模拟" : ""}
                </td>
                <td>{labels[job.stage] ?? job.stage}</td>
                <td>
                  {job.verified_chunks} /{" "}
                  {artifact?.manifest.hashes.length ?? "?"}
                </td>
                <td>{labels[job.detail] ?? job.detail}</td>
                <td>
                  {["queued", "downloading", "ready"].includes(job.stage) ? (
                    <button
                      disabled={disabled || busy || !access.operate}
                      onClick={() => {
                        void onMutate(
                          "distribution/cancel",
                          { job_id: job.id },
                          "已取消后续下载与发布。",
                        ).catch((error) =>
                          setError(
                            error instanceof Error ? error.message : "取消失败",
                          ),
                        );
                      }}
                    >
                      取消
                    </button>
                  ) : (
                    <button
                      disabled={disabled || busy || !access.operate}
                      onClick={() => {
                        setArtifactId(job.artifact_id);
                        setSelected([job.device_id]);
                        setPreview(null);
                      }}
                    >
                      选择此设备补发
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}
