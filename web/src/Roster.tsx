import { useAccess } from "./access";
import { useState } from "react";
import { z } from "zod";
import { api, type Mutation } from "./api";
import type { Snapshot } from "./contracts";
import { DetailsDialog } from "./DetailsDialog";
import { seatStatusLabels, matchesSearch } from "./fleet";
import {
  changeLabels,
  rosterPreviewSchema,
  sampleRoster,
  type Contest,
  type Participant,
} from "./roster-model";

const historySchema = z.array(
  z.object({
    sequence: z.number(),
    request_id: z.string(),
    version: z.number(),
    created_at: z.number(),
    result: rosterPreviewSchema,
  }),
);
type Preview = z.infer<typeof rosterPreviewSchema>;
export function Roster({
  snapshot,
  contest,
  csrf,
  disabled,
  onMutate,
}: {
  snapshot: Snapshot;
  contest?: Contest;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const access = useAccess();
  const [text, setText] = useState("");
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [roomIds, setRoomIds] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [prepared, setPrepared] = useState<{
    input: Record<string, unknown>;
    preview: Preview;
    attempted: boolean;
  } | null>(null);
  const [editing, setEditing] = useState<
    (Participant & { base_version: number }) | null
  >(null);
  const [target, setTarget] = useState("");
  const [seatAction, setSeatAction] = useState("move");
  const [handoff, setHandoff] = useState<string[]>([]);
  const [history, setHistory] = useState<z.infer<typeof historySchema>>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const seats = snapshot.seating.rooms.flatMap((room) =>
    room.seats.map((seat) => ({ room, seat })),
  );
  function location(id: string | null, rooms = snapshot.seating.rooms) {
    for (const room of rooms) {
      const seat = room.seats.find((seat) => seat.id === id);
      if (seat) return `${room.name} / ${seat.number}`;
    }
    return id ? "座位已移除" : "未分配";
  }
  async function preview(
    operation: Record<string, unknown>,
    createId?: string,
    editVersion?: number,
  ) {
    setLoading(true);
    setError("");
    setPrepared(null);
    const input = {
      request_id: crypto.randomUUID(),
      contest_id: createId ?? contest?.id,
      base_version: createId
        ? snapshot.roster.version
        : (editVersion ?? contest?.version),
      operation,
    };
    try {
      const value = rosterPreviewSchema.parse(
        await api("roster/preview", input, csrf),
      );
      setPrepared({ input, preview: value, attempted: false });
    } catch (error) {
      setError(error instanceof Error ? error.message : "预览失败");
    } finally {
      setLoading(false);
    }
  }
  async function commit() {
    if (!prepared) return;
    setPrepared({ ...prepared, attempted: true });
    setLoading(true);
    setError("");
    try {
      await onMutate(
        "roster/commit",
        { ...prepared.input, preview_token: prepared.preview.preview_token },
        `操作已保存，请求编号 ${String(prepared.input.request_id)}。可在操作历史核对结果。`,
      );
      setHandoff(
        prepared.preview.changes
          .filter((change) => change.before?.seat_id !== change.after.seat_id)
          .map((change) => change.after.id),
      );
      setPrepared(null);
      setEditing(null);
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "提交失败，可使用原请求重试",
      );
    } finally {
      setLoading(false);
    }
  }
  async function readHistory(more = false) {
    if (!contest) return;
    setLoading(true);
    setError("");
    try {
      const result = historySchema.parse(
        await api(
          `roster/${contest.id}/history${more && history.length ? `?before=${history.at(-1)!.sequence}` : ""}`,
        ),
      );
      setHistory(more ? [...history, ...result] : result);
      setHistoryOpen(true);
    } catch (error) {
      setError(error instanceof Error ? error.message : "历史加载失败");
    } finally {
      setLoading(false);
    }
  }
  const rows =
    contest?.participants.filter(
      (p) =>
        matchesSearch(
          [p.code, p.name, p.school, location(p.seat_id)],
          search,
        ) &&
        (filter === "all" || (filter === "assigned") === !!p.seat_id),
    ) ?? [];
  const stale =
    prepared &&
    ((prepared.input.contest_id !== contest?.id &&
      prepared.input.operation &&
      (prepared.input.operation as { kind: string }).kind !== "create") ||
      prepared.preview.base_version !==
        ((prepared.input.operation as { kind: string }).kind === "create"
          ? snapshot.roster.version
          : contest?.version));
  const locked = disabled || loading;
  return (
    <div className="fleet-page roster-page">
      {handoff.length > 0 && (
        <p className="panel" role="status">
          本地分配已保存，涉及 {handoff.length} 名选手。评测账号尚需核对：
          <a
            href={`#/domjudge?participants=${encodeURIComponent(handoff.join(","))}`}
          >
            继续核对这些选手的 IP 与旧会话
          </a>
        </p>
      )}
      <section className="panel">
        <details>
          <summary>创建比赛</summary>
          <form
            className="device-form"
            onSubmit={(event) => {
              event.preventDefault();
              const data = new FormData(event.currentTarget);
              void preview(
                { kind: "create", name: String(data.get("name")) },
                String(data.get("id")),
              );
            }}
          >
            <label>
              比赛 ID
              <input
                name="id"
                required
                pattern="[A-Za-z0-9_-]+"
                maxLength={80}
              />
            </label>
            <label>
              比赛名称
              <input name="name" required maxLength={100} />
            </label>
            <button disabled={locked}>预览创建比赛</button>
          </form>
        </details>
        <p>
          每场独立保存名单和分配。切换比赛后，座位页及设备详情同步显示所选场次。历史可按请求编号核对。
        </p>
      </section>
      {error && (
        <p role="alert" className="banner error">
          {error} · 可重试，版本变化时请重新预览。
        </p>
      )}
      {loading && <p role="status">正在处理…</p>}
      {contest ? (
        <>
          <section className="panel">
            <div className="section-heading">
              <h2>导入名单</h2>
              <button
                onClick={() => {
                  setText(sampleRoster);
                  setPrepared(null);
                }}
                disabled={locked}
              >
                填入合成样例
              </button>
            </div>
            <p>
              UTF-8 TSV（可含 BOM），表头 code / name /
              school，以制表符分隔。三项必填，首尾空白去除，内部空格及中文保留。标识区分大小写，保留前导零。缺失行保留，重复导入按同场标识更新。
            </p>
            <label>
              上传 TSV
              <input
                type="file"
                accept=".tsv,.txt"
                disabled={locked}
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  if (!file) return;
                  setPrepared(null);
                  if (file.size > 100000) {
                    setError("文件最多 100000 字节");
                    return;
                  }
                  void file
                    .arrayBuffer()
                    .then((buffer) => {
                      setText(
                        new TextDecoder("utf-8", { fatal: true }).decode(
                          buffer,
                        ),
                      );
                      setError("");
                    })
                    .catch(() => setError("编码无效，请保存为 UTF-8"));
                }}
              />
            </label>
            <label>
              名单内容
              <textarea
                rows={6}
                value={text}
                disabled={locked}
                onChange={(event) => {
                  setText(event.target.value);
                  setPrepared(null);
                }}
              />
            </label>
            <button
              className="primary"
              disabled={locked || !text}
              onClick={() => void preview({ kind: "import", text })}
            >
              预览导入差异
            </button>
          </section>
          <section className="panel">
            <h2>自动分配</h2>
            <p>
              按参赛标识 UTF-8 字节顺序处理全部未分配选手。机房按稳定 ID
              字节顺序、座位按显示编号递增。保留已有分配，跳过已占座、地址冲突和禁用设备座位。未注册、无
              IP 及离线座位允许提前安排，容量不足整批拒绝。
            </p>
            <div className="actions">
              {snapshot.seating.rooms.map((room) => (
                <label key={room.id}>
                  <input
                    type="checkbox"
                    checked={roomIds.includes(room.id)}
                    disabled={locked}
                    onChange={(event) => {
                      setRoomIds(
                        event.target.checked
                          ? [...roomIds, room.id]
                          : roomIds.filter((id) => id !== room.id),
                      );
                      setPrepared(null);
                    }}
                  />
                  {room.name} · {room.id}
                </label>
              ))}
            </div>
            <button
              disabled={locked || !roomIds.length}
              onClick={() => void preview({ kind: "auto", room_ids: roomIds })}
            >
              预览自动分配
            </button>
          </section>
          <section className="panel">
            <div className="section-heading">
              <h2>
                {contest.name} · {contest.participants.length} 名选手 · v
                {contest.version}
              </h2>
              <button
                disabled={loading || !access.allRooms}
                onClick={() => void readHistory()}
              >
                操作历史
              </button>
            </div>
            <div className="filter-bar">
              <label>
                搜索选手
                <input
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder="标识、姓名、学校或座位"
                />
              </label>
              <label>
                分配筛选
                <select
                  value={filter}
                  onChange={(event) => setFilter(event.target.value)}
                >
                  <option value="all">全部</option>
                  <option value="assigned">已分配</option>
                  <option value="unassigned">未分配</option>
                </select>
              </label>
            </div>
            <div className="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>参赛标识</th>
                    <th>姓名</th>
                    <th>学校</th>
                    <th>座位</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((p) => (
                    <tr key={p.id}>
                      <td>{p.code}</td>
                      <td>{p.name}</td>
                      <td>{p.school}</td>
                      <td>{location(p.seat_id)}</td>
                      <td>
                        <button
                          disabled={locked}
                          onClick={() => {
                            setEditing({ ...p, base_version: contest.version });
                            setTarget("");
                            setSeatAction("move");
                            setPrepared(null);
                          }}
                        >
                          编辑与分配
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {!rows.length && (
              <p className="empty">
                {contest.participants.length
                  ? "没有符合筛选的选手。"
                  : "尚无名单，请先导入并核对预览。"}
              </p>
            )}
          </section>
        </>
      ) : (
        <p className="empty">请创建或在页面顶部选择比赛。</p>
      )}
      {prepared && (
        <section className="panel" aria-label="操作预览">
          <h2>核对预览 · {prepared.preview.name}</h2>
          <p>
            基准 v{prepared.preview.base_version} · 提交后共{" "}
            {prepared.preview.total} 名选手
          </p>
          {stale && (
            <p role="alert" className="error">
              {prepared.attempted
                ? "版本已变化，已尝试提交的请求可原样重试以核对是否成功，也可查看操作历史。重新编辑前请重新预览。"
                : "版本已变化，请重新预览。"}
            </p>
          )}
          {prepared.preview.errors.map((error, index) => (
            <p className="error" role="alert" key={index}>
              {error.line ? `第 ${error.line} 行 · ` : ""}
              {error.message}
            </p>
          ))}
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>行 / 差异</th>
                  <th>提交前</th>
                  <th>提交后</th>
                  <th>座位影响</th>
                </tr>
              </thead>
              <tbody>
                {prepared.preview.changes.map((change, index) => (
                  <tr key={index}>
                    <td>
                      {change.line || "—"} · {changeLabels[change.kind]}
                    </td>
                    <td>
                      {change.before
                        ? `${change.before.code} · ${change.before.name} · ${change.before.school}`
                        : "新增"}
                    </td>
                    <td>
                      {change.after.code} · {change.after.name} ·{" "}
                      {change.after.school}
                    </td>
                    <td>
                      {location(
                        change.before?.seat_id ?? null,
                        prepared.preview.rooms,
                      )}{" "}
                      → {location(change.after.seat_id, prepared.preview.rooms)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!!prepared.preview.skipped.length && (
            <details>
              <summary>跳过 {prepared.preview.skipped.length} 个座位</summary>
              {prepared.preview.skipped.map((item) => (
                <p key={item.seat_id}>
                  {location(item.seat_id, prepared.preview.rooms)} ·{" "}
                  {item.reason === "occupied"
                    ? "本场已分配"
                    : "地址冲突或设备禁用"}
                </p>
              ))}
            </details>
          )}
          <div className="actions">
            <button
              className="primary"
              disabled={
                locked ||
                (!!stale && !prepared.attempted) ||
                !!prepared.preview.errors.length
              }
              onClick={() => void commit()}
            >
              确认提交 / 重试原请求
            </button>
            <button
              disabled={locked}
              onClick={() =>
                void preview(
                  prepared.input.operation as Record<string, unknown>,
                  (prepared.input.operation as { kind: string }).kind ===
                    "create"
                    ? String(prepared.input.contest_id)
                    : undefined,
                )
              }
            >
              重新预览当前状态
            </button>
            <button disabled={loading} onClick={() => setPrepared(null)}>
              取消预览
            </button>
          </div>
        </section>
      )}
      {editing && contest && (
        <DetailsDialog
          title={`${editing.code} · 编辑与分配`}
          onClose={() => setEditing(null)}
        >
          <p>
            稳定身份 {editing.id}，当前座位 {location(editing.seat_id)}
            。资料修改会同步显示在该座位及匹配设备详情。
          </p>
          {(
            [
              ["code", "参赛标识"],
              ["name", "姓名"],
              ["school", "学校"],
            ] as const
          ).map(([key, label]) => (
            <label key={key}>
              {label}
              <input
                value={editing[key]}
                disabled={locked}
                onChange={(event) =>
                  setEditing({ ...editing, [key]: event.target.value })
                }
              />
            </label>
          ))}
          <button
            disabled={locked}
            onClick={() => {
              void preview(
                {
                  kind: "edit",
                  participant_id: editing.id,
                  code: editing.code,
                  name: editing.name,
                  school: editing.school,
                },
                undefined,
                editing.base_version,
              );
              setEditing(null);
            }}
          >
            预览资料修正
          </button>
          <label>
            座位操作
            <select
              value={seatAction}
              disabled={locked}
              onChange={(e) => {
                setSeatAction(e.target.value);
                setTarget("");
              }}
            >
              <option value="move">移到空座</option>
              <option value="swap" disabled={!editing.seat_id}>
                与另一名选手交换
              </option>
              <option value="remove" disabled={!editing.seat_id}>
                解除本人的座位分配
              </option>
            </select>
          </label>
          <label>
            目标座位
            <select
              value={target}
              disabled={locked}
              onChange={(event) => setTarget(event.target.value)}
            >
              <option value="">
                {seatAction === "remove" ? "解除分配" : "请选择目标座位"}
              </option>
              {seats
                .filter(
                  ({ seat }) =>
                    seatAction !== "remove" &&
                    seat.id !== editing.seat_id &&
                    contest.participants.some((p) => p.seat_id === seat.id) ===
                      (seatAction === "swap"),
                )
                .map(({ room, seat }) => (
                  <option key={seat.id} value={seat.id}>
                    {room.name} / {seat.number} ·{" "}
                    {contest.participants.find((p) => p.seat_id === seat.id)
                      ?.code ?? "空座位"}{" "}
                    · {seatStatusLabels[seat.status]}
                  </option>
                ))}
            </select>
          </label>
          <p>
            {editing.name}：{location(editing.seat_id)} →{" "}
            {target
              ? location(target)
              : seatAction === "remove"
                ? "未分配"
                : "请选择座位"}
            。
            {seatAction === "swap" &&
              target &&
              `${contest.participants.find((p) => p.seat_id === target)?.name} 将移至 ${location(editing.seat_id)}。`}{" "}
            若只是原位换电脑，请到{" "}
            <a href="#/rooms" onClick={() => setEditing(null)}>
              机房设置
            </a>{" "}
            核对占用。
          </p>
          <button
            disabled={locked || (!target && seatAction !== "remove")}
            onClick={() => {
              const other = contest.participants.find(
                (p) => p.seat_id === target && p.id !== editing.id,
              );
              const changes = [
                { participant_id: editing.id, seat_id: target || null },
              ];
              if (other)
                changes.push({
                  participant_id: other.id,
                  seat_id: editing.seat_id,
                });
              void preview(
                { kind: "assign", changes },
                undefined,
                editing.base_version,
              );
              setEditing(null);
            }}
          >
            预览调整或交换
          </button>
        </DetailsDialog>
      )}
      {historyOpen && (
        <DetailsDialog
          title="名单与分配操作历史"
          onClose={() => setHistoryOpen(false)}
        >
          {history.map((event) => (
            <details key={event.sequence}>
              <summary>
                v{event.version} ·{" "}
                {new Date(event.created_at * 1000).toLocaleString("zh-CN")} ·{" "}
                {event.request_id}
              </summary>
              {event.result.changes.map((change, index) => (
                <p key={index}>
                  {changeLabels[change.kind]} ·{" "}
                  {change.before
                    ? `${change.before.code} · ${change.before.name} · ${change.before.school}`
                    : "新增"}{" "}
                  → {change.after.code} · {change.after.name} ·{" "}
                  {change.after.school} ·{" "}
                  {location(change.before?.seat_id ?? null, event.result.rooms)}{" "}
                  → {location(change.after.seat_id, event.result.rooms)}
                </p>
              ))}
            </details>
          ))}
          {!history.length && <p>暂无操作历史</p>}
          <button
            disabled={loading || !history.length}
            onClick={() => void readHistory(true)}
          >
            加载更早记录
          </button>
        </DetailsDialog>
      )}
    </div>
  );
}
