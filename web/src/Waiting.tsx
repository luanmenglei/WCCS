import { useEffect, useRef, useState } from "react";
import { z } from "zod";
import { api } from "./api";
import type { Snapshot } from "./contracts";
import { Seats } from "./Seats";
import type { Mutation } from "./SeatChecks";
import { DetailsDialog } from "./DetailsDialog";
import {
  actualLabel,
  accountLabel,
  actionLabels,
  type ControlAction,
  commandSchema,
  skipLabels,
  stageLabels,
  waitingListSchema,
  waitingPreviewSchema,
  type WaitingItem,
} from "./waiting-model";

type Request = {
  request_id: string;
  contest_id: string;
  seat_ids: string[];
  action: ControlAction;
  confirm_session_end?: boolean;
  duration_seconds: number;
  preview_token?: string;
};
export function Waiting({
  snapshot,
  contestId,
  selected,
  onSelect,
  csrf,
  disabled,
  onMutate,
  mode = "waiting",
}: {
  mode?: "waiting" | "login";
  snapshot: Snapshot;
  contestId: string;
  selected: string[];
  onSelect: (ids: string[]) => void;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const [action, setAction] = useState<ControlAction>(
    mode === "login" ? "prepare_login" : "enter",
  );
  const [confirmSessionEnd, setConfirmSessionEnd] = useState(false);
  const loginMode = mode === "login";
  const [duration, setDuration] = useState(900);
  const [preview, setPreview] = useState<z.infer<
    typeof waitingPreviewSchema
  > | null>(null);
  const [request, setRequest] = useState<Request | null>(null);
  const [reviewed, setReviewed] = useState("");
  const [items, setItems] = useState<WaitingItem[]>([]);
  const [detail, setDetail] = useState<WaitingItem | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [fresh, setFresh] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const sequence = useRef(0);
  const selection = JSON.stringify([
    contestId,
    selected,
    action,
    duration,
    confirmSessionEnd,
  ]);
  const current = reviewed === selection ? preview : null;
  useEffect(() => {
    const requestSequence = sequence;
    const owner = ++requestSequence.current;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const value = waitingListSchema.parse(
          await api(
            contestId
              ? `waiting?contest_id=${encodeURIComponent(contestId)}`
              : "waiting",
          ),
        );
        if (owner !== requestSequence.current) return;
        setItems(value.items);
        setFresh(true);
      } catch {
        if (owner === requestSequence.current) setFresh(false);
      }
      if (owner === requestSequence.current)
        timer = setTimeout(() => void poll(), 2000);
    }
    void poll();
    return () => {
      requestSequence.current++;
      clearTimeout(timer);
    };
  }, [refresh, contestId]);
  async function run(operation: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await operation();
    } catch (error) {
      setError(error instanceof Error ? error.message : "等待控制失败");
    } finally {
      setBusy(false);
    }
  }
  const contest = snapshot.roster.contests.find((c) => c.id === contestId);
  const visible = items.filter(
    (i) => i.command.identity.contest_id === contestId,
  );
  const latest = visible.filter(
    (item, index) =>
      visible.findIndex(
        (i) => i.command.device_id === item.command.device_id,
      ) === index,
  );
  const retryable = latest.filter(
    (i) =>
      ["failed", "expired", "cancelled"].includes(i.stage) ||
      !i.online ||
      i.actual?.state === "failed",
  );
  return (
    <>
      <p>
        {loginMode
          ? "准备并登录需核对普通比赛账户、当前会话、工具访问及离线恢复入口。设备状态标明文件模拟或 Windows 测试适配器。断线撤回限制并保留已登录桌面与文件，重连后需重新预览确认。"
          : "管理员与操作员可控制授权机房。重新核对读取当前状态。重试需重新预览身份和目标，进入等待最长一小时，到期恢复桌面。"}
      </p>
      {!contest && <p className="banner">请在顶部选择比赛。</p>}
      <Seats
        contest={contest}
        snapshot={snapshot}
        selected={selected}
        onSelect={onSelect}
        csrf={csrf}
        disabled={disabled}
        onMutate={onMutate}
      >
        <section className="check-review" aria-label="等待控制操作">
          <div className="actions">
            <label>
              {loginMode ? "账户操作" : "等待操作"}
              <select
                value={action}
                onChange={(e) => setAction(e.target.value as ControlAction)}
                disabled={busy}
              >
                {loginMode ? (
                  <>
                    <option value="prepare_login">准备并统一登录</option>
                    <option value="restore_teaching">显式恢复教学账户</option>
                  </>
                ) : (
                  <>
                    <option value="enter">进入等待界面</option>
                    <option value="leave">结束等待，恢复普通桌面</option>
                  </>
                )}
              </select>
            </label>
            <label>
              有效期（秒）
              <input
                type="number"
                min={30}
                max={3600}
                value={duration}
                disabled={busy}
                onChange={(e) => setDuration(Number(e.target.value))}
              />
            </label>
            <button
              className="primary"
              disabled={
                disabled || busy || !fresh || !contest || !selected.length
              }
              onClick={() =>
                void run(async () => {
                  const body: Request = {
                    request_id: crypto.randomUUID(),
                    contest_id: contestId,
                    seat_ids: [...selected],
                    action,
                    duration_seconds: duration,
                    ...(action === "prepare_login"
                      ? { confirm_session_end: confirmSessionEnd }
                      : {}),
                  };
                  const value = waitingPreviewSchema.parse(
                    await api("waiting/preview", body, csrf),
                  );
                  setRequest({ ...body, preview_token: value.preview_token });
                  setPreview(value);
                  setReviewed(selection);
                  setNotice("");
                })
              }
            >
              {loginMode ? "预览登录与恢复目标" : "预览等待目标"}（
              {selected.length}）
            </button>
          </div>
          {loginMode && action === "prepare_login" && (
            <label>
              <input
                type="checkbox"
                checked={confirmSessionEnd}
                disabled={busy}
                onChange={(e) => setConfirmSessionEnd(e.target.checked)}
              />
              已核对所选教学会话保存工作，同意在准备过程中退出这些会话。
            </label>
          )}
          {error && (
            <p className="banner error" role="alert">
              {error}
            </p>
          )}
          {notice && (
            <p className="banner" role="status">
              {notice}
            </p>
          )}
          {current && request && (
            <>
              <h3>
                {current.contest_name} · {actionLabels[action]}
              </h3>
              <p>
                {current.targets.filter((t) => t.status === "ready").length}{" "}
                台可执行 · 比赛版本 {current.contest_version} · 有效期{" "}
                {duration} 秒
              </p>
              <div className="table-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>机房 / 座位</th>
                      <th>选手快照</th>
                      <th>设备 / 身份代次</th>
                      <th>目标会话</th>
                      {loginMode && <th>账户 / 恢复条件</th>}
                      <th>处理</th>
                    </tr>
                  </thead>
                  <tbody>
                    {current.targets.map((t) => (
                      <tr key={t.seat_id}>
                        <td>
                          {t.room_name} / {t.number}
                        </td>
                        <td>
                          {t.identity
                            ? `${t.identity.name} · ${t.identity.school}`
                            : "未分配"}
                        </td>
                        <td>
                          {t.device_id ?? "未匹配"} / {t.generation ?? "未知"}
                        </td>
                        <td>{t.session_key ?? "不可用"}</td>
                        {loginMode && (
                          <td>
                            {accountLabel(t.account)}
                            <br />
                            目标 SID {t.account?.target_sid ?? "待创建"}
                            <br />
                            会话 {t.account?.session ?? "未知"} · 工具{" "}
                            {t.account?.tools_ready ? "可用" : "待重新核验"} ·
                            恢复入口{" "}
                            {t.account?.recovery_ready ? "就绪" : "未就绪"}
                          </td>
                        )}
                        <td>{skipLabels[t.status]}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {loginMode && current.tool_paths && (
                <p>
                  工具配置版本 {current.tool_version} ·{" "}
                  {current.tool_paths.join("、") || "尚未配置工具路径"}
                </p>
              )}
              {loginMode && (
                <p>
                  {action === "prepare_login"
                    ? "执行步骤为核对恢复入口、核对账户所有权与工具、保存原状态、按确认结束教学会话、应用临时限制、提交一次性登录并核对桌面。失败后撤回本轮修改。"
                    : "关闭等待窗口并撤回本轮账户限制，保留当前桌面、程序及文件。恢复冲突需现场处理。"}
                </p>
              )}
              <div className="actions">
                <button
                  className="primary"
                  disabled={
                    disabled ||
                    busy ||
                    !fresh ||
                    !current.targets.some((t) => t.status === "ready")
                  }
                  onClick={() =>
                    void run(async () => {
                      const result = z
                        .object({
                          request_id: z.string(),
                          commands: z.array(commandSchema),
                        })
                        .parse(await api("waiting/commit", request, csrf));
                      setNotice(
                        `已持久提交 ${result.commands.length} 台，等待逐机接收及完成。请求 ${result.request_id}`,
                      );
                      setPreview(null);
                      setRefresh((v) => v + 1);
                    })
                  }
                >
                  {busy
                    ? "正在提交…"
                    : loginMode
                      ? `确认批量${actionLabels[action]}`
                      : action === "enter"
                        ? "确认批量进入等待"
                        : "确认批量结束等待"}
                </button>
                <button disabled={busy} onClick={() => setPreview(null)}>
                  关闭预览
                </button>
              </div>
              <p className="mono">
                请求 {request.request_id} · 提交失败可用此预览重试原请求。
              </p>
            </>
          )}
        </section>
      </Seats>
      <section>
        <div className="actions">
          <h2>逐机实际状态</h2>
          <button onClick={() => setRefresh((v) => v + 1)}>重新核对</button>
          <button
            disabled={disabled || !retryable.length}
            onClick={() => {
              onSelect(retryable.map((i) => i.command.identity.seat_id));
              setPreview(null);
              setNotice("已选中异常目标，请核对操作并重新预览。");
            }}
          >
            选择异常目标重新预览
          </button>
        </div>
        {!fresh && (
          <p className="banner error">
            控制状态读取失败，显示上次记录，写入暂停。
          </p>
        )}
        <p>
          {latest.filter((i) => i.stage === "succeeded").length} /{" "}
          {latest.length} 台命令完成 · {latest.filter((i) => !i.online).length}{" "}
          台离线 · {retryable.length} 台待核对
        </p>
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>座位 / 选手快照</th>
                <th>设备</th>
                <th>期望</th>
                <th>命令阶段</th>
                <th>当前实际状态</th>
                <th>详情</th>
              </tr>
            </thead>
            <tbody>
              {latest.map((item) => (
                <tr key={item.command.command_id}>
                  <td>
                    {item.command.identity.seat_label}
                    <br />
                    {item.command.identity.name} ·{" "}
                    {item.command.identity.school}
                  </td>
                  <td>{item.command.device_id}</td>
                  <td>{actionLabels[item.command.action]}</td>
                  <td>{stageLabels[item.stage]}</td>
                  <td>
                    {fresh ? actualLabel(item) : "状态未知"}
                    <br />
                    {item.actual?.detail}
                    {fresh && item.online && item.actual?.account && (
                      <>
                        <br />
                        {accountLabel(item.actual.account)}
                      </>
                    )}
                  </td>
                  <td>
                    <button onClick={() => setDetail(item)}>
                      查看执行详情
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!latest.length && <p>当前比赛暂无控制命令。</p>}
        {loginMode && <a href="#/waiting">进入等待控制</a>}
      </section>
      {detail && (
        <DetailsDialog
          title="控制命令与实际结果"
          onClose={() => setDetail(null)}
        >
          <p>命令 {detail.command.command_id}</p>
          <p>请求 {detail.command.request_id}</p>
          <p>
            身份代次 {detail.command.generation} · 设备版本{" "}
            {detail.command.device_version} · 序号 {detail.command.sequence}
          </p>
          <p>
            到期时间{" "}
            {new Date(detail.command.expires_at * 1000).toLocaleString()}
          </p>
          <p>
            阶段 {stageLabels[detail.stage]} · {actualLabel(detail)}
          </p>
          <p>
            完成证据 {detail.report?.actual.detail ?? "尚未回传"} · 屏幕{" "}
            {detail.report?.actual.monitors ?? 0}
          </p>
          <p>
            {accountLabel(detail.actual?.account)} · 实际 SID{" "}
            {detail.actual?.account?.observed_sid ?? "未确认"} · 桌面回执{" "}
            {detail.actual?.account?.desktop_confirmed ? "已确认" : "未确认"}
          </p>
          <a
            href={`#/events?request_id=${encodeURIComponent(detail.command.request_id)}`}
          >
            查询关联事件
          </a>
        </DetailsDialog>
      )}
    </>
  );
}
