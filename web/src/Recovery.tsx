import { useEffect, useMemo, useState } from "react";
import { z } from "zod";
import { api, type Mutation } from "./api";
import type { Snapshot } from "./contracts";
import { maintenanceReasons, maintenanceStages } from "./maintenance-model";

const limitsSchema = z.object({
  max_entries: z.number().int().min(1).max(100000),
  archive_bytes: z
    .number()
    .int()
    .min(0)
    .max(512 * 1024 * 1024),
  timeout_seconds: z.number().int().min(60).max(86400),
});
const optionsSchema = z.object({
  limits: limitsSchema.optional(),
  action: z.string(),
  contest_id: z.string(),
  target: z.string(),
  scope: z.string(),
  extensions: z.array(z.string()),
  skip_reason: z.string(),
  previous_request: z.string().nullable(),
  confirm_session_end: z.boolean(),
});
const manifestSchema = z.object({
  target: z.string(),
  volume: z.object({
    guid: z.string(),
    serial: z.string(),
    label: z.string(),
    system: z.boolean(),
  }),
  files: z.array(z.object({ path: z.string(), size: z.number() })),
  directories: z.array(z.string()),
});
const jobSchema = z.object({
  job_id: z.string(),
  request_id: z.string(),
  device_id: z.string(),
  device_name: z.string(),
  template: z.object({ options: optionsSchema }),
  stage: z.string(),
  expired: z.boolean(),
  archive_verified: z.boolean(),
  reboot_observed: z.boolean(),
  reboot_requested_at: z.number().nullable(),
  reboot_observed_at: z.number().nullable(),
  file_count: z.number().nullable(),
  directory_count: z.number().nullable(),
  total_bytes: z.string().nullable(),
  result: z
    .object({
      detail: z.string(),
      recovery: manifestSchema.optional(),
      steps: z.array(z.object({ detail: z.string() })).default([]),
    })
    .nullable(),
});
const listSchema = z.object({ jobs: z.array(jobSchema) });
const previewSchema = z.object({
  preview_token: z.string(),
  targets: z.array(
    z.object({
      device_id: z.string(),
      name: z.string(),
      identity: z.object({ seat_label: z.string() }),
      reason: z.string().nullable(),
      recovery: z
        .object({
          job_id: z.string(),
          volume: manifestSchema.shape.volume,
          collected_count: z.number(),
          file_count: z.number(),
          directory_count: z.number(),
          total_bytes: z.string(),
        })
        .nullable(),
    }),
  ),
});
const actions: Record<string, string> = {
  inspect: "检查文件与卷",
  archive: "归档或记录跳过",
  clean: "清理批准范围",
  reboot: "恢复后重启",
};
const details: Record<string, string> = {
  recovery_inspected: "文件与卷已检查",
  archive_verified: "服务器已核验归档",
  archive_skipped: "已记录跳过归档",
  cleanup_complete: "批准范围已清理",
  reboot_requested: "已请求重启，等待重新上线",
  recovery_started: "正在检查并执行",
  recovery_volume_not_provisioned: "请现场核对 D 盘，并在客户端设置部署卷身份",
  recovery_volume_policy_invalid: "请核对客户端部署卷配置格式",
  recovery_volume_identity_mismatch: "设备或 D 盘身份与部署配置不一致",
  recovery_target_changed: "文件或卷发生变化，请重新检查",
  recovery_authorization_ended: "连接或授权结束，请重新检查",
  target_reparse_point: "范围内存在链接或挂载点，请现场核对",
  recovery_capacity_exceeded:
    "超出本次文件数量或归档容量上限，请调整配置后重新检查",
  restore_teaching_required: "请先恢复教学并核对策略已撤回",
  recovery_io_or_transfer_failed:
    "文件占用、权限或传输失败，请保存并关闭程序后重新检查",
  execution_failed: "执行失败",
  restart_interrupted: "服务重启中断，请重新检查剩余文件",
};
type Job = z.infer<typeof jobSchema>;
function JobFiles({
  job,
  contestId,
  extensions,
}: {
  job: Pick<Job, "job_id" | "file_count" | "directory_count" | "total_bytes">;
  contestId: string;
  extensions: string[];
}) {
  const [page, setPage] = useState<z.infer<typeof manifestSchema> | null>(null);
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  async function load(start: number) {
    setLoading(true);
    try {
      const data = await api(
        `recovery/manifest/${encodeURIComponent(job.job_id)}?contest_id=${encodeURIComponent(contestId)}&offset=${start}`,
      );
      setPage(manifestSchema.parse(data));
      setOffset(start);
      setError("");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }
  const count = Math.max(job.file_count ?? 0, job.directory_count ?? 0);
  return (
    <div>
      <button
        disabled={loading}
        onClick={() => (page ? setPage(null) : void load(0))}
      >
        {page ? "收起清单" : "查看清单"}（{job.file_count} 个文件，
        {job.directory_count} 个目录，
        {BigInt(job.total_bytes ?? "0").toLocaleString()} 字节）
      </button>
      {error && <p role="alert">{error}</p>}
      {page && (
        <>
          <p>
            {page.target} · {page.volume.guid} · {page.volume.serial}
          </p>
          <ul>
            {page.files.map((f) => (
              <li key={f.path}>
                {f.path} · {f.size} 字节 ·{" "}
                {extensions.some((ext) =>
                  f.path.toLowerCase().endsWith("." + ext.toLowerCase()),
                )
                  ? "归档"
                  : "仅清理"}
              </li>
            ))}
            {page.directories.map((d) => (
              <li key={d}>{d}/</li>
            ))}
          </ul>
          <button
            disabled={loading || offset === 0}
            onClick={() => void load(offset - 100)}
          >
            上一页
          </button>
          <button
            disabled={loading || offset + 100 >= count}
            onClick={() => void load(offset + 100)}
          >
            下一页
          </button>
        </>
      )}
    </div>
  );
}
export function Recovery({
  snapshot,
  contestId,
  csrf,
  disabled,
  onMutate,
}: {
  snapshot: Snapshot;
  contestId: string;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const [selected, setSelected] = useState<string[]>([]);
  const [room, setRoom] = useState("");
  const roomDevices = new Set(
    snapshot.seating.rooms
      .filter((item) => item.id === room)
      .flatMap((item) => item.seats.flatMap((seat) => seat.devices)),
  );
  const visible = snapshot.devices.filter(
    (device) => !room || roomDevices.has(device.id),
  );
  const [options, setOptions] = useState<z.infer<typeof optionsSchema>>({
    limits: {
      max_entries: 100000,
      archive_bytes: 512 * 1024 * 1024,
      timeout_seconds: 3600,
    },
    action: "inspect",
    contest_id: contestId,
    target: "C:\\WCCS-Work",
    scope: "workspace",
    extensions: ["cpp", "c", "h", "hpp", "py", "java", "pas"],
    skip_reason: "",
    previous_request: null,
    confirm_session_end: false,
  });
  const [extensionText, setExtensionText] = useState(
    options.extensions.join(" "),
  );
  const [skip, setSkip] = useState(false);
  const extensions = useMemo(
    () => (skip ? [] : extensionText.trim().split(/\s+/).filter(Boolean)),
    [skip, extensionText],
  );
  const [jobs, setJobs] = useState<Job[]>([]);
  const [batchFilter, setBatchFilter] = useState(
    () =>
      new URLSearchParams(location.hash.split("?")[1]).get("request_id") ?? "",
  );
  const [batchInput, setBatchInput] = useState(batchFilter);
  const [fresh, setFresh] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [reviewed, setReviewed] = useState("");
  const [preview, setPreview] = useState<z.infer<typeof previewSchema> | null>(
    null,
  );
  const [request, setRequest] = useState<Record<string, unknown> | null>(null);
  const selection = JSON.stringify([
    contestId,
    selected,
    options,
    skip,
    extensionText,
  ]);
  const current = selection === reviewed ? preview : null;
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        const value = listSchema.parse(
          await api(
            `recovery?contest_id=${encodeURIComponent(contestId)}&${batchFilter ? "request_id=" + encodeURIComponent(batchFilter) : ""}`,
          ),
        );
        if (active) {
          setJobs(
            value.jobs.filter(
              (j) => j.template.options.contest_id === contestId,
            ),
          );
          setFresh(true);
        }
      } catch {
        if (active) setFresh(false);
      }
      if (active) timer = setTimeout(() => void refresh(), 2000);
    }
    void refresh();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [contestId, batchFilter]);
  async function review() {
    setBusy(true);
    setError("");
    setPreview(null);
    const body = {
      request_id: crypto.randomUUID(),
      template_version: 1,
      targets: selected,
      template: {
        operation: "recovery",
        options: {
          ...options,
          contest_id: contestId,
          extensions,
          skip_reason: skip ? options.skip_reason : "",
        },
      },
    };
    try {
      const value = previewSchema.parse(
        await api("maintenance/preview", body, csrf),
      );
      setPreview(value);
      setRequest({ ...body, preview_token: value.preview_token });
      setReviewed(selection);
    } catch (e) {
      setError(e instanceof Error ? e.message : "预览失败");
    } finally {
      setBusy(false);
    }
  }
  const [batchNote, setBatchNote] = useState("");
  async function next(job: Job) {
    const previous = job.template.options;
    const action = (
      { inspect: "archive", archive: "clean", clean: "reboot" } as Record<
        string,
        string
      >
    )[previous.action];
    if (!action || previous.contest_id !== contestId) return;
    setBusy(true);
    setPreview(null);
    setRequest(null);
    setBatchNote("");
    setError("");
    try {
      const batch = listSchema.parse(
        await api(
          `recovery?contest_id=${encodeURIComponent(contestId)}&request_id=${encodeURIComponent(job.request_id)}`,
        ),
      ).jobs;
      const successes = batch.filter((item) => item.stage === "succeeded");
      if (!successes.length) throw new Error("本批没有可继续的成功设备。");
      setBatchNote(
        `按当前权限核对本批 ${batch.length} 台，已选成功 ${successes.length} 台，未进入下一步 ${batch.length - successes.length} 台。`,
      );
      setExtensionText(previous.extensions.join(" "));
      setOptions({
        ...previous,
        action,
        previous_request: job.request_id,
        confirm_session_end: false,
      });
      setSkip(previous.extensions.length === 0);
      setSelected(successes.map((item) => item.device_id));
    } catch (error) {
      setError(error instanceof Error ? error.message : "批次读取失败");
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <section className="panel operations-panel">
        <h2>赛后归档与恢复</h2>
        <p>
          使用重启复原镜像的机房，收场时可直接
          <a href="#/maintenance?operation=shutdown">统一关机</a>
          。以下工具供需要归档或清理文件时使用。
        </p>
        {batchNote && <p role="status">{batchNote}</p>}
        <p>
          检查 → 归档或明确跳过 → 恢复教学 → 清理 → 重启 →
          再次检查。文件变化或任务中断后，从检查步骤重新核对。
        </p>
        <p>
          清理总字节数不限。归档容量仅统计选定扩展名的文件。 整卷清理保留
          Windows 的回收站与 System Volume Information 系统目录。
        </p>
        <label>
          当前步骤
          <input readOnly value={actions[options.action]} />
        </label>
        <label>
          清理范围
          <select
            value={options.scope}
            disabled={options.action !== "inspect"}
            onChange={(e) =>
              setOptions({
                ...options,
                scope: e.target.value,
                target: e.target.value === "volume" ? "D:\\" : "C:\\WCCS-Work",
              })
            }
          >
            <option value="workspace">已配置的比赛工作目录</option>
            <option value="volume">部署目标 D 盘整卷</option>
          </select>
        </label>
        {options.limits && (
          <fieldset disabled={options.action !== "inspect"}>
            <legend>本次容量与执行时间</legend>
            <label>
              文件及目录数量上限
              <input
                type="number"
                min={1}
                max={100000}
                value={options.limits.max_entries}
                onChange={(e) =>
                  setOptions({
                    ...options,
                    limits: {
                      ...options.limits!,
                      max_entries: Number(e.target.value),
                    },
                  })
                }
              />
            </label>
            <label>
              归档上限（MiB，最大 512）
              <input
                type="number"
                min={0}
                max={512}
                value={options.limits.archive_bytes / 1048576}
                onChange={(e) =>
                  setOptions({
                    ...options,
                    limits: {
                      ...options.limits!,
                      archive_bytes: Number(e.target.value) * 1048576,
                    },
                  })
                }
              />
            </label>
            <label>
              执行时限（分钟，1～1440）
              <input
                type="number"
                min={1}
                max={1440}
                value={options.limits.timeout_seconds / 60}
                onChange={(e) =>
                  setOptions({
                    ...options,
                    limits: {
                      ...options.limits!,
                      timeout_seconds: Number(e.target.value) * 60,
                    },
                  })
                }
              />
            </label>
          </fieldset>
        )}
        <label>
          实际路径
          <input
            value={options.target}
            disabled={
              options.action !== "inspect" || options.scope === "volume"
            }
            onChange={(e) => setOptions({ ...options, target: e.target.value })}
          />
        </label>
        <label>
          <input
            type="checkbox"
            checked={skip}
            disabled={options.action !== "inspect"}
            onChange={(e) => setSkip(e.target.checked)}
          />
          本场明确跳过代码归档
        </label>
        {skip ? (
          <label>
            跳过原因
            <input
              value={options.skip_reason}
              disabled={options.action !== "inspect"}
              onChange={(e) =>
                setOptions({ ...options, skip_reason: e.target.value })
              }
            />
          </label>
        ) : (
          <label>
            收集扩展名，空格分隔
            <input
              value={extensionText}
              disabled={options.action !== "inspect"}
              onChange={(e) => setExtensionText(e.target.value)}
            />
          </label>
        )}
        <p>清理会删除所选范围内全部文件。归档仅收集上面列出的扩展名。</p>
        <label>
          按机房查看
          <select value={room} onChange={(e) => setRoom(e.target.value)}>
            <option value="">全部授权设备</option>
            {snapshot.seating.rooms.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name}
              </option>
            ))}
          </select>
        </label>
        {options.action === "inspect" && (
          <button
            disabled={disabled || busy || !room || !visible.length}
            onClick={() =>
              setSelected([
                ...new Set([
                  ...selected,
                  ...visible.map((device) => device.id),
                ]),
              ])
            }
          >
            追加本机房设备
          </button>
        )}
        <p>
          已选 {selected.length} 台，其中{" "}
          {
            selected.filter((id) => !visible.some((device) => device.id === id))
              .length
          }{" "}
          台不在当前视图。
        </p>
        <div className="target-list">
          {visible.map((device) => (
            <label key={device.id}>
              <input
                type="checkbox"
                checked={selected.includes(device.id)}
                onChange={(e) =>
                  setSelected(
                    e.target.checked
                      ? [...selected, device.id]
                      : selected.filter((id) => id !== device.id),
                  )
                }
              />
              {device.name} · {device.id}
            </label>
          ))}
        </div>
        {["clean", "reboot"].includes(options.action) && (
          <label>
            <input
              type="checkbox"
              checked={options.confirm_session_end}
              onChange={(e) =>
                setOptions({
                  ...options,
                  confirm_session_end: e.target.checked,
                })
              }
            />
            我已确认选手保存文件并结束使用，同意清理上述范围及本步骤涉及的会话结束。
          </label>
        )}
        <div className="actions">
          <button
            disabled={disabled || busy || !selected.length || !contestId}
            onClick={() => void review()}
          >
            预览本步骤
          </button>
          <button
            disabled={busy}
            onClick={() => {
              setOptions({
                ...options,
                action: "inspect",
                previous_request: null,
                confirm_session_end: false,
              });
              setPreview(null);
            }}
          >
            重新检查
          </button>
          <a href={`#/login?contest_id=${encodeURIComponent(contestId)}`}>
            恢复教学账户
          </a>
        </div>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        {current && (
          <>
            <h3>
              {snapshot.roster.contests.find((c) => c.id === contestId)?.name ??
                contestId}{" "}
              · 本次目标 {current.targets.length} 台
            </h3>
            {current.targets.map((target) => (
              <div key={target.device_id}>
                <h4>
                  {target.identity.seat_label} · {target.name} ·{" "}
                  {target.reason
                    ? (maintenanceReasons[target.reason] ??
                      details[target.reason] ??
                      target.reason)
                    : "可执行"}
                </h4>
                {target.recovery && (
                  <>
                    <p>
                      {options.target} ·{" "}
                      {target.recovery.volume.system
                        ? "系统卷内的指定目录"
                        : "非系统卷"}{" "}
                      · {target.recovery.volume.label || "无卷标"} · 序列号{" "}
                      {target.recovery.volume.serial} ·{" "}
                      {target.recovery.volume.guid}
                    </p>
                    <p>
                      所选扩展名匹配 {target.recovery.collected_count}{" "}
                      个归档文件。
                    </p>
                    <JobFiles
                      key={target.recovery.job_id}
                      job={target.recovery}
                      contestId={contestId}
                      extensions={extensions}
                    />
                  </>
                )}
              </div>
            ))}
            <button
              className="primary"
              disabled={
                disabled ||
                busy ||
                !request ||
                !current.targets.some((t) => !t.reason)
              }
              onClick={() => {
                if (!request) return;
                setBusy(true);
                void onMutate(
                  "maintenance/commit",
                  request,
                  "赛后任务已下发，请核对逐机结果。",
                )
                  .then(() => {
                    setPreview(null);
                    setRequest(null);
                  })
                  .catch((e) =>
                    setError(e instanceof Error ? e.message : "提交失败"),
                  )
                  .finally(() => setBusy(false));
              }}
            >
              确认执行{actions[options.action]}
            </button>
          </>
        )}
      </section>
      <section className="panel">
        <h2>逐机恢复结果</h2>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (batchFilter !== batchInput.trim()) {
              setJobs([]);
              setFresh(false);
              setBatchFilter(batchInput.trim());
            }
          }}
        >
          <label>
            按批次请求编号查找
            <input
              value={batchInput}
              onChange={(event) => setBatchInput(event.target.value)}
              placeholder="留空显示最近 500 条"
            />
          </label>
          <button>查看批次</button>
        </form>
        {fresh && !jobs.length && <p>没有匹配的可见任务。</p>}
        {!fresh && <p role="status">结果连接中断，当前显示数据已过期。</p>}
        {jobs.map((job) => (
          <article key={job.job_id}>
            <h3>
              {job.device_name} · {actions[job.template.options.action]} ·{" "}
              {job.template.options.action === "reboot" &&
              job.stage === "succeeded" &&
              !job.reboot_observed
                ? "请求已受理"
                : (maintenanceStages[job.stage] ?? job.stage)}
            </h3>
            <p>
              {details[job.result?.detail ?? ""] ??
                job.result?.detail ??
                (job.expired ? "任务已过期，请重新检查" : "等待回传")}
            </p>
            <small>批次：{job.request_id}</small>
            {job.result?.steps.map((step, index) => (
              <p key={index}>{details[step.detail] ?? step.detail}</p>
            ))}
            {job.template.options.action === "reboot" && (
              <p>
                {job.reboot_requested_at &&
                  `请求受理于 ${new Date(job.reboot_requested_at * 1000).toLocaleString()}。`}
                {job.reboot_observed_at &&
                  `首次核对于 ${new Date(job.reboot_observed_at * 1000).toLocaleString()}。`}
                {job.reboot_observed
                  ? "已观察到重启后的教学状态，请再次检查复原后的工作目录。"
                  : "尚未核对重启后的教学状态。"}
              </p>
            )}
            {job.file_count !== null && (
              <JobFiles
                job={job}
                contestId={contestId}
                extensions={job.template.options.extensions}
              />
            )}
            <div className="actions">
              {job.stage === "succeeded" &&
                job.template.options.action !== "reboot" && (
                  <button
                    disabled={disabled || busy || !fresh}
                    onClick={() => next(job)}
                  >
                    为本批成功设备准备下一步
                  </button>
                )}
              {job.archive_verified && (
                <a
                  href={`/api/recovery/archives/${encodeURIComponent(job.job_id)}`}
                >
                  下载归档 ZIP 与清单
                </a>
              )}
              <a
                href={`#/events?request_id=${encodeURIComponent(job.request_id)}`}
              >
                查看事件与处置
              </a>
              {["queued", "accepted", "executing"].includes(job.stage) && (
                <button
                  disabled={disabled || busy}
                  onClick={() =>
                    void onMutate(
                      "maintenance/cancel",
                      { job_id: job.job_id },
                      "已请求停止，请核对实际结果。",
                    ).catch((e) => setError(String(e)))
                  }
                >
                  停止后续执行
                </button>
              )}
            </div>
          </article>
        ))}
      </section>
    </>
  );
}
