import { useEffect, useState } from "react";
import { z } from "zod";
import { api } from "./api";
import { useAccess } from "./access";
import type { Snapshot } from "./contracts";
import type { Mutation } from "./SeatChecks";
import { Seats } from "./Seats";
import { skipLabels } from "./waiting-model";

const previewSchema = z.object({
  preview_token: z.string(),
  remote: z.object({ phase: z.string(), revision: z.number() }),
  desktop: z.object({
    targets: z.array(
      z.object({
        seat_id: z.string(),
        room_name: z.string(),
        number: z.number(),
        status: z.string(),
      }),
    ),
  }),
});
const jobsSchema = z.object({
  jobs: z.array(
    z.object({
      id: z.string(),
      stage: z.string(),
      execute_at: z.number(),
      detail: z.string(),
      payload: z.object({
        contest_id: z.string(),
        remote_contest_id: z.string(),
        action: z.string(),
      }),
      results: z
        .object({
          remote: z
            .union([z.string(), z.object({ phase: z.string() }).passthrough()])
            .optional(),
          reports: z
            .array(z.object({ device_id: z.string(), stage: z.string() }))
            .optional(),
        })
        .passthrough(),
    }),
  ),
});
const actions: Record<string, string> = {
  start: "开始比赛",
  pause: "暂停比赛",
  resume: "恢复比赛",
  end: "结束比赛",
};
const stages: Record<string, string> = {
  scheduled: "等待执行",
  running: "联动中",
  succeeded: "联动完成",
  failed: "需要处理",
  cancelled: "已取消",
  interrupted: "服务重启后待核对",
};
const phases: Record<string, string> = {
  scheduled: "尚未开始",
  running: "比赛中",
  paused: "已暂停",
  ended: "已结束",
  pending: "结果待核对",
};
const details: Record<string, string> = {
  target_changed_review_required: "目标、连接或身份已变化，请重新预览",
  desktop_failed_review_required: "部分桌面执行失败，请核对逐机结果",
  desktop_timeout_review_required: "桌面回执超时，请核对实际状态",
  server_restarted_review_required: "服务重启已中断安排，请重新核对并操作",
  operator_cancelled: "管理员已取消安排",
  remote_connection_failed: "无法连接 DOMjudge，请核对两端状态后重试",
  remote_timeout: "DOMjudge 超时，请核对实际状态后重试",
};

export function ContestControl({
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
  const access = useAccess();
  const [selected, setSelected] = useState<string[]>([]);
  const [remoteContest, setRemoteContest] = useState("");
  const [action, setAction] = useState("pause");
  const [at, setAt] = useState("");
  const [duration, setDuration] = useState(900);
  const [preview, setPreview] = useState<z.infer<typeof previewSchema> | null>(
    null,
  );
  const [requestId, setRequestId] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [jobs, setJobs] = useState<z.infer<typeof jobsSchema>["jobs"]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!access.allRooms) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        const value = jobsSchema.parse(await api("contest-control"));
        if (active) setJobs(value.jobs);
      } catch (error) {
        if (active)
          setError(error instanceof Error ? error.message : "读取失败");
      }
      if (active) timer = setTimeout(() => void refresh(), 1000);
    }
    void refresh();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [access.allRooms]);
  function invalidate() {
    setPreview(null);
    setConfirmed(false);
  }
  async function run(operation: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await operation();
    } catch (error) {
      setError(error instanceof Error ? error.message : "操作失败");
    } finally {
      setBusy(false);
    }
  }
  const input = {
    contest_id: contestId,
    remote_contest_id: remoteContest,
    seat_ids: selected,
    action,
    execute_at: at ? Math.floor(new Date(at).getTime() / 1000) : 0,
    duration_seconds: duration,
  };
  if (!access.allRooms) return <p role="alert">比赛联动需要全场查看权限。</p>;
  return (
    <section className="panel operations-panel">
      <h2>比赛安排与联动</h2>
      <p>
        暂停会停止 DOMjudge
        计时和提交，并让所选桌面进入等待。恢复会先核对桌面解除结果，再恢复计时和提交。
      </p>
      <p>
        断线或服务重启会中断原安排，联网后需要重新预览操作。失败时先核对下方两端结果。
      </p>
      {error && <p role="alert">{error}</p>}
      <fieldset disabled={disabled || busy || !access.manage}>
        <label>
          DOMjudge 比赛 ID
          <input
            value={remoteContest}
            onChange={(event) => {
              setRemoteContest(event.target.value);
              invalidate();
            }}
          />
        </label>
        <label>
          操作
          <select
            value={action}
            onChange={(event) => {
              setAction(event.target.value);
              invalidate();
            }}
          >
            {Object.entries(actions).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label>
          安排时间（留空立即执行）
          <input
            type="datetime-local"
            value={at}
            onChange={(event) => {
              setAt(event.target.value);
              invalidate();
            }}
          />
        </label>
        <label>
          桌面等待有效秒数
          <input
            type="number"
            min={30}
            max={3600}
            value={duration}
            onChange={(event) => {
              setDuration(Number(event.target.value));
              invalidate();
            }}
          />
        </label>
        <Seats
          contest={snapshot.roster.contests.find(
            (contest) => contest.id === contestId,
          )}
          csrf={csrf}
          disabled={disabled || busy}
          onMutate={onMutate}
          targetMode
          snapshot={snapshot}
          selected={selected}
          onSelect={(ids) => {
            setSelected(ids);
            invalidate();
          }}
        >
          {false}
        </Seats>
        <button
          disabled={!contestId || !remoteContest || selected.length === 0}
          onClick={() =>
            void run(async () => {
              invalidate();
              const id = crypto.randomUUID();
              setRequestId(id);
              setPreview(
                previewSchema.parse(
                  await api(
                    "contest-control/preview",
                    { ...input, request_id: id },
                    csrf,
                  ),
                ),
              );
            })
          }
        >
          预览比赛与桌面
        </button>
        {preview && (
          <>
            <p>
              DOMjudge 当前
              {phases[preview.remote.phase] ?? preview.remote.phase}，版本{" "}
              {preview.remote.revision}。
            </p>
            <table>
              <thead>
                <tr>
                  <th>座位</th>
                  <th>状态</th>
                </tr>
              </thead>
              <tbody>
                {preview.desktop.targets.map((target) => (
                  <tr key={target.seat_id}>
                    <td>
                      {target.room_name} / {target.number}
                    </td>
                    <td>
                      {target.status === "ready"
                        ? "可执行"
                        : ((skipLabels as Record<string, string>)[
                            target.status
                          ] ?? target.status)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <label>
              <input
                type="checkbox"
                checked={confirmed}
                onChange={(event) => setConfirmed(event.target.checked)}
              />
              已核对比赛及全部目标，确认同时操作 DOMjudge 和桌面
            </label>
            <button
              disabled={
                !confirmed ||
                preview.desktop.targets.some(
                  (target) => target.status !== "ready",
                )
              }
              onClick={() =>
                void run(async () => {
                  await onMutate(
                    "contest-control/commit",
                    {
                      ...input,
                      request_id: requestId,
                      preview_token: preview.preview_token,
                    },
                    "比赛联动任务已创建，请核对两端结果",
                  );
                  invalidate();
                })
              }
            >
              确认{actions[action]}
            </button>
          </>
        )}
      </fieldset>
      <h3>操作结果</h3>
      <table>
        <thead>
          <tr>
            <th>比赛与操作</th>
            <th>时间</th>
            <th>进度</th>
            <th>DOMjudge</th>
            <th>桌面</th>
            <th>处理</th>
          </tr>
        </thead>
        <tbody>
          {jobs
            .filter((job) => job.payload.contest_id === contestId)
            .map((job) => (
              <tr key={job.id}>
                <td>
                  {job.payload.remote_contest_id} /{" "}
                  {actions[job.payload.action]}
                </td>
                <td>{new Date(job.execute_at * 1000).toLocaleString()}</td>
                <td>
                  {stages[job.stage]}
                  {job.detail && <p>{details[job.detail] ?? job.detail}</p>}
                </td>
                <td>
                  {typeof job.results.remote === "object"
                    ? phases[job.results.remote.phase]
                    : (phases[job.results.remote ?? ""] ?? "尚未执行")}
                </td>
                <td>
                  {job.results.reports?.map((report) => (
                    <div key={report.device_id}>
                      {report.device_id} · {report.stage}
                    </div>
                  )) ?? "尚无回执"}
                </td>
                <td>
                  {job.stage === "scheduled" && (
                    <button
                      disabled={disabled || busy || !access.manage}
                      onClick={() =>
                        void run(async () => {
                          await onMutate(
                            "contest-control/cancel",
                            {
                              request_id: crypto.randomUUID(),
                              job_id: job.id,
                            },
                            "比赛安排已取消",
                          );
                        })
                      }
                    >
                      取消安排
                    </button>
                  )}
                </td>
              </tr>
            ))}
        </tbody>
      </table>
    </section>
  );
}
