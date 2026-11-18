import { useEffect, useState } from "react";
import { z } from "zod";
import { api } from "./api";
import { useAccess } from "./access";
import type { Snapshot } from "./contracts";
import type { Mutation } from "./SeatChecks";

const targetSchema = z
  .object({
    participant_id: z.string(),
    code: z.string(),
    name: z.string(),
    username: z.string(),
    seat_label: z.string().optional(),
    ip: z.string().nullable().optional(),
    reason: z.string().nullable(),
    revoke_only: z.boolean().optional(),
    remote_user: z
      .object({ ip: z.string().nullable(), enabled: z.boolean() })
      .passthrough()
      .nullable(),
  })
  .passthrough();
const previewSchema = z.object({
  preview_token: z.string(),
  targets: z.array(targetSchema),
});
const stateSchema = z.object({
  configured: z.boolean(),
  url: z.string().nullable(),
  bindings: z.array(
    z.object({
      participant_id: z.string(),
      remote_contest_id: z.string(),
      username: z.string(),
      ip: z.string(),
    }),
  ),
  jobs: z.array(
    z.object({
      id: z.string(),
      stage: z.string(),
      detail: z.string(),
      plan: z.object({
        contest_id: z.string(),
        remote_contest_id: z.string(),
        group_id: z.string(),
        targets: z.array(targetSchema),
      }),
      results: z.array(
        z.object({
          participant_id: z.string(),
          stage: z.string(),
          detail: z.string(),
        }),
      ),
    }),
  ),
});

const details: Record<string, string> = {
  unassigned_account_revoked: "未分配选手的旧账号与 IP 已撤销",
  seat_unassigned: "尚未分配座位",
  seat_not_ready: "座位离线、冲突或尚未绑定",
  device_disabled: "设备已禁用",
  remote_account_conflict: "远端账号身份或权限冲突",
  remote_ip_conflict: "IP 已被其他账号使用",
  remote_account_changed: "远端账号已变化，请重新预览",
  remote_team_conflict: "远端队伍 ID 冲突",
  local_target_changed: "名单或设备状态已变化，请重新核对",
  operator_authorization_changed: "操作员授权已变化",
  remote_timeout: "DOMjudge 超时，请重新预览核对实际结果",
  remote_connection_failed: "无法连接 DOMjudge",
  remote_response_lost: "响应丢失，请重新预览核对实际结果",
  local_database_unavailable: "本地数据库暂时不可用",
  remote_team_not_in_contest: "队伍未加入目标比赛，账号已停用",
  remote_ip_conflict_after_write: "发现并发 IP 冲突，账号已停用",
  target_changed_remote_recovery_required:
    "目标已变化且远端恢复未确认，请先在 DOMjudge 停用该账号",
  server_restarted_review_required: "服务重启中断任务，请重新预览核对实际结果",
  checking_remote_account: "核对远端账号",
  remote_result_pending: "等待远端结果，失败后需重新核对",
  old_session_revoked: "旧网页会话已撤销",
  account_ip_verified: "账号和 IP 已核对",
  already_synced: "绑定一致，无需更新",
};
function detail(value: string) {
  return (
    details[value] ??
    (/^remote_http_\d+$/.test(value)
      ? `DOMjudge 返回 HTTP ${value.slice(12)}`
      : value)
  );
}
const stages: Record<string, string> = {
  queued: "排队",
  running: "执行中",
  succeeded: "完成",
  failed: "失败",
  interrupted: "已中断",
  checking: "核对中",
  revoking: "撤销旧绑定中",
  revoked: "旧绑定已撤销",
  activating: "新绑定结果待核对",
};

export function Domjudge({
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
  const [state, setState] = useState<z.infer<typeof stateSchema> | null>(null);
  const contest = snapshot.roster.contests.find(
    (item) => item.id === contestId,
  );
  const remoteContest = contest?.remote_contest_id ?? "";
  const [candidate, setCandidate] = useState("");
  const [choices, setChoices] = useState<{ id: string; name: string }[]>([]);
  const [group, setGroup] = useState("participants");
  const [selected, setSelected] = useState<string[]>(() => {
    const ids =
      new URLSearchParams(globalThis.location?.hash.split("?")[1])
        .get("participants")
        ?.split(",") ?? [];
    return (
      contest?.participants
        .filter((p) => ids.includes(p.id))
        .map((p) => p.id) ?? []
    );
  });
  const seatMap = new Map(
    snapshot.seating.rooms.flatMap((room) =>
      room.seats.map((seat) => [seat.id, seat] as const),
    ),
  );
  const [preview, setPreview] = useState<z.infer<typeof previewSchema> | null>(
    null,
  );
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const participants =
    snapshot.roster.contests.find((contest) => contest.id === contestId)
      ?.participants ?? [];
  useEffect(() => {
    if (!access.allRooms) return;
    let active = true;
    let pending = false;
    async function refresh() {
      if (pending) return;
      pending = true;
      try {
        const value = stateSchema.parse(await api("domjudge"));
        if (active) setState(value);
      } catch (error) {
        if (active)
          setError(error instanceof Error ? error.message : "读取失败");
      } finally {
        pending = false;
      }
    }
    void refresh();
    const timer = setInterval(() => void refresh(), 3000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [access.allRooms]);
  function invalidate() {
    setPreview(null);
    setConfirmed(false);
  }
  const input = {
    contest_id: contestId,
    remote_contest_id: remoteContest,
    group_id: group,
    participant_ids: selected,
  };
  if (!access.allRooms)
    return <p role="alert">DOMjudge 账号核对需要全场查看权限。</p>;
  return (
    <section className="panel operations-panel">
      <h2>DOMjudge 账号与换机</h2>
      {state && (
        <p>
          {state.configured
            ? `已配置 ${state.url}`
            : "服务器尚未配置 DOMjudge API 连接材料。"}
        </p>
      )}
      <fieldset
        disabled={disabled || busy || !access.manage || !state?.configured}
      >
        <p>
          本地比赛：{contest?.name ?? "请先选择当前比赛"} · 已绑定的评测比赛：
          {remoteContest || "未绑定"}
        </p>
        <details open={!remoteContest || undefined}>
          <summary>设置评测比赛绑定</summary>
          <button
            disabled={!contestId}
            onClick={async () => {
              setBusy(true);
              setError("");
              try {
                setChoices(
                  z
                    .object({
                      contests: z.array(
                        z.object({ id: z.string(), name: z.string() }),
                      ),
                    })
                    .parse(await api("domjudge/choices")).contests,
                );
              } catch (error) {
                setError(
                  error instanceof Error ? error.message : "读取比赛失败",
                );
              } finally {
                setBusy(false);
              }
            }}
          >
            读取评测端比赛列表
          </button>
          <label>
            目标评测比赛
            <select
              value={candidate}
              onChange={(event) => setCandidate(event.target.value)}
            >
              <option value="">选择名称和编号</option>
              {choices.map((choice) => (
                <option key={choice.id} value={choice.id}>
                  {choice.name} · {choice.id}
                </option>
              ))}
            </select>
          </label>
          <button
            disabled={!candidate || !contestId || candidate === remoteContest}
            onClick={async () => {
              if (
                !window.confirm(
                  `将「${contest?.name}」绑定到「${choices.find((c) => c.id === candidate)?.name} · ${candidate}」？账号同步和整场计时都将使用此绑定。`,
                )
              )
                return;
              setBusy(true);
              setError("");
              try {
                await onMutate(
                  "domjudge/bind",
                  {
                    contest_id: contestId,
                    previous: remoteContest || null,
                    remote_contest_id: candidate,
                  },
                  "比赛绑定已保存",
                );
                invalidate();
              } catch (error) {
                setError(error instanceof Error ? error.message : "绑定失败");
              } finally {
                setBusy(false);
              }
            }}
          >
            确认绑定两场比赛
          </button>
        </details>
        <label>
          该比赛的分组 ID
          <input
            value={group}
            onChange={(event) => {
              setGroup(event.target.value);
              invalidate();
            }}
          />
        </label>
        <button
          onClick={() => {
            setSelected(participants.map((participant) => participant.id));
            invalidate();
          }}
        >
          选择本场全部选手
        </button>
        <table>
          <thead>
            <tr>
              <th>选择</th>
              <th>参赛标识</th>
              <th>姓名</th>
              <th>已记录的 DOMjudge 绑定</th>
            </tr>
          </thead>
          <tbody>
            {participants.map((participant) => {
              const binding = state?.bindings.find(
                (item) => item.participant_id === participant.id,
              );
              return (
                <tr key={participant.id}>
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`选择 ${participant.code}`}
                      checked={selected.includes(participant.id)}
                      onChange={(event) => {
                        setSelected(
                          event.target.checked
                            ? [...selected, participant.id]
                            : selected.filter((id) => id !== participant.id),
                        );
                        invalidate();
                      }}
                    />
                  </td>
                  <td>{participant.code}</td>
                  <td>{participant.name}</td>
                  <td>
                    {binding
                      ? `${binding.remote_contest_id} / ${binding.ip}`
                      : "尚未同步"}
                    {binding &&
                      (binding.remote_contest_id !== remoteContest ||
                        binding.ip !==
                          seatMap.get(participant.seat_id ?? "")
                            ?.expected_ip) && (
                        <strong>
                          {" "}
                          ·{" "}
                          {participant.seat_id
                            ? "座位 IP 待核对"
                            : "旧绑定待撤销"}
                        </strong>
                      )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <button
          disabled={
            !contestId || !remoteContest || !group || selected.length === 0
          }
          onClick={() => {
            setBusy(true);
            setError("");
            invalidate();
            void api(
              "domjudge/preview",
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
          核对账号与座位 IP
        </button>
        {preview && (
          <>
            <table>
              <thead>
                <tr>
                  <th>选手</th>
                  <th>座位</th>
                  <th>旧 IP</th>
                  <th>本次 IP</th>
                  <th>核对结果</th>
                </tr>
              </thead>
              <tbody>
                {preview.targets.map((target) => (
                  <tr key={target.participant_id}>
                    <td>
                      {target.code} · {target.name}
                    </td>
                    <td>{target.seat_label ?? "未分配"}</td>
                    <td>{target.remote_user?.ip ?? "无"}</td>
                    <td>{target.ip ?? "无"}</td>
                    <td>
                      {target.reason
                        ? detail(target.reason)
                        : target.revoke_only
                          ? "仅撤销旧账号及 IP"
                          : "可同步"}
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
              已核对所选选手与座位，允许更新绑定并撤销旧电脑上的 DOMjudge
              网页会话
            </label>
            <button
              disabled={
                !confirmed ||
                preview.targets.some((target) => target.reason !== null)
              }
              onClick={() => {
                setBusy(true);
                setError("");
                void onMutate(
                  "domjudge/commit",
                  { ...input, preview_token: preview.preview_token },
                  "DOMjudge 同步任务已创建，请核对逐人结果。",
                )
                  .then(invalidate)
                  .catch((error) =>
                    setError(
                      error instanceof Error ? error.message : "提交失败",
                    ),
                  )
                  .finally(() => setBusy(false));
              }}
            >
              应用账号绑定
            </button>
          </>
        )}
      </fieldset>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {state?.jobs
        .filter((job) => job.plan.contest_id === contestId)
        .map((job) => (
          <details
            key={job.id}
            open={
              job.stage === "running" ||
              job.stage === "failed" ||
              job.stage === "interrupted"
            }
          >
            <summary>
              {stages[job.stage] ?? job.stage} · {job.plan.remote_contest_id} ·{" "}
              {job.id}
            </summary>
            {job.detail && <p>{detail(job.detail)}</p>}
            <table>
              <thead>
                <tr>
                  <th>选手</th>
                  <th>状态</th>
                  <th>结果</th>
                </tr>
              </thead>
              <tbody>
                {job.plan.targets.map((target) => {
                  const result = job.results.find(
                    (item) => item.participant_id === target.participant_id,
                  );
                  return (
                    <tr key={target.participant_id}>
                      <td>
                        {target.code} · {target.name}
                      </td>
                      <td>
                        {result
                          ? (stages[result.stage] ?? result.stage)
                          : "尚未执行"}
                      </td>
                      <td>{result ? detail(result.detail) : ""}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {(job.stage === "failed" || job.stage === "interrupted") && (
              <button
                disabled={busy}
                onClick={() => {
                  setSelected(
                    job.plan.targets.map((target) => target.participant_id),
                  );
                  if (job.plan.remote_contest_id !== remoteContest) {
                    setError("历史任务的比赛与当前绑定不同");
                    return;
                  }
                  setGroup(job.plan.group_id);
                  invalidate();
                }}
              >
                重新选择这些选手并核对
              </button>
            )}
          </details>
        ))}
    </section>
  );
}
