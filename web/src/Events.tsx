import { useEffect, useRef, useState } from "react";
import { z } from "zod";
import { api } from "./api";
import { DetailsDialog } from "./DetailsDialog";

export const eventSchema = z.object({
  handling: z
    .object({
      version: z.number(),
      status: z.enum(["open", "acknowledged", "resolved"]),
      note: z.string(),
      actor_id: z.string(),
      updated_at: z.number(),
    })
    .nullable()
    .optional(),
  id: z.string(),
  source: z.string(),
  source_id: z.string(),
  category: z.string(),
  occurred_at: z.number(),
  actor_id: z.string().nullable(),
  actor_name: z.string().nullable(),
  operation: z.string(),
  target: z.string().nullable(),
  device_ids: z.array(z.string()),
  room_ids: z.array(z.string()).nullable(),
  scope_kind: z.enum(["rooms", "global", "unknown"]),
  contest_id: z.string().nullable(),
  request_id: z.string().nullable(),
  version: z.number().nullable(),
  outcome: z.string(),
  detail: z.unknown(),
  legacy: z.boolean(),
});
const pageSchema = z.object({
  items: z.array(eventSchema),
  next_cursor: z.string().nullable(),
});
type Event = z.infer<typeof eventSchema>;
function scopeLabel(event: Event) {
  return event.scope_kind === "global"
    ? "全场操作"
    : event.scope_kind === "rooms"
      ? event.room_ids?.join("、")
      : "未知机房范围";
}
function actorLabel(event: Event) {
  if (event.actor_name) return event.actor_name;
  if (event.category === "device_report" || event.category === "task_result")
    return "设备 / 任务回报";
  return event.category === "system" ? "系统" : "操作者未知";
}
const fields = [
  ["actor_id", "操作者稳定 ID"],
  ["operation", "事件类型"],
  ["device_id", "设备 ID"],
  ["room_id", "机房 ID"],
  ["contest_id", "比赛 ID"],
  ["request_id", "请求编号"],
] as const;
export function eventQuery(values: Record<string, string>) {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(values))
    if (value.trim()) {
      if (key === "from" || key === "to") {
        const time = Date.parse(value);
        if (!Number.isFinite(time)) throw new Error("时间筛选格式无效");
        params.set(key, String(Math.floor(time / 1000)));
      } else params.set(key, value.trim());
    }
  return params.toString();
}
const categories: Record<string, string> = {
  management: "管理动作",
  device_report: "设备上报",
  task: "任务下发",
  task_result: "任务结果",
  system: "系统事件",
  historical: "历史来源",
};
const outcomes: Record<string, string> = {
  warning: "设备变化告警",
  succeeded: "成功",
  failed: "执行失败",
  accepted: "设备已接收",
  executing: "执行中",
  expired: "已过期",
  cancelled: "已取消",
  pending: "等待结果",
  recorded: "已记录",
  unknown: "未知",
};

export function Events({
  csrf,
  disabled,
}: {
  csrf: string;
  disabled: boolean;
}) {
  const [note, setNote] = useState("");
  const [handling, setHandling] = useState("acknowledged");
  const [saving, setSaving] = useState(false);
  const [filters, setFilters] = useState<Record<string, string>>(() => ({
    request_id:
      new URLSearchParams(location.hash.split("?")[1]).get("request_id") ?? "",
  }));
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<Event[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");
  const [detail, setDetail] = useState<Event | null>(null);
  const [retry, setRetry] = useState<{ query: string; cursor?: string } | null>(
    null,
  );
  const sequence = useRef(0);
  const detailsSequence = useRef(0);
  async function load(nextQuery: string, nextCursor?: string) {
    const owner = ++sequence.current;
    setLoading(true);
    setError("");
    setRetry({ query: nextQuery, cursor: nextCursor });
    try {
      const params = new URLSearchParams(nextQuery);
      if (nextCursor) params.set("cursor", nextCursor);
      const result = pageSchema.parse(await api(`events?${params}`));
      if (owner !== sequence.current) return;
      setItems((old) =>
        nextCursor ? [...old, ...result.items] : result.items,
      );
      setCursor(result.next_cursor);
      setQuery(nextQuery);
      setLoaded(true);
      setRetry(null);
    } catch (error) {
      if (owner === sequence.current)
        setError(error instanceof Error ? error.message : "事件读取失败");
    } finally {
      if (owner === sequence.current) setLoading(false);
    }
  }
  useEffect(() => {
    const requestSequence = sequence;
    const detailSequence = detailsSequence;
    const request = new URLSearchParams(location.hash.split("?")[1]).get(
      "request_id",
    );
    void load(
      request ? new URLSearchParams({ request_id: request }).toString() : "",
    );
    return () => {
      requestSequence.current++;
      detailSequence.current++;
    };
  }, []);
  async function inspect(id: string) {
    const owner = ++detailsSequence.current;
    setError("");
    try {
      const value = eventSchema.parse(
        await api(`events/${encodeURIComponent(id)}`),
      );
      if (owner === detailsSequence.current) {
        setDetail(value);
        setNote(value.handling?.note ?? "");
        setHandling(value.handling?.status ?? "acknowledged");
      }
    } catch (error) {
      if (owner === detailsSequence.current)
        setError(error instanceof Error ? error.message : "详情读取失败");
    }
  }
  return (
    <section className="panel">
      <form
        className="event-filters"
        onSubmit={(e) => {
          e.preventDefault();
          try {
            void load(eventQuery(filters));
          } catch (error) {
            setError(String(error));
          }
        }}
      >
        <label>
          开始时间（含）
          <input
            type="datetime-local"
            value={filters.from ?? ""}
            onChange={(e) => setFilters({ ...filters, from: e.target.value })}
          />
        </label>
        <label>
          结束时间（含）
          <input
            type="datetime-local"
            value={filters.to ?? ""}
            onChange={(e) => setFilters({ ...filters, to: e.target.value })}
          />
        </label>
        {fields.map(([key, label]) => (
          <label key={key}>
            {label}
            <input
              value={filters[key] ?? ""}
              onChange={(e) =>
                setFilters({ ...filters, [key]: e.target.value })
              }
            />
          </label>
        ))}
        <label>
          事件来源
          <select
            value={filters.category ?? ""}
            onChange={(e) =>
              setFilters({ ...filters, category: e.target.value })
            }
          >
            <option value="">全部可见来源</option>
            {Object.entries(categories).map(([key, label]) => (
              <option key={key} value={key}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label>
          结果
          <select
            value={filters.outcome ?? ""}
            onChange={(e) =>
              setFilters({ ...filters, outcome: e.target.value })
            }
          >
            <option value="">全部结果</option>
            {Object.entries(outcomes).map(([key, label]) => (
              <option key={key} value={key}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <div className="actions">
          <button className="primary" disabled={loading}>
            查询事件
          </button>
          <button
            type="button"
            disabled={loading}
            onClick={() => {
              setFilters({});
              void load("");
            }}
          >
            清空筛选
          </button>
        </div>
      </form>
      <p>
        按服务器时间及事件 ID
        倒序。翻页保持本次查询快照，重新查询读取新事件。未知历史范围仅向全场授权身份开放。
      </p>
      {loading && <p role="status">正在查询事件…</p>}
      {error && (
        <div role="alert" className="error">
          {error}
          {retry && (
            <button
              disabled={loading}
              onClick={() => void load(retry.query, retry.cursor)}
            >
              重试查询
            </button>
          )}
        </div>
      )}
      {loaded && !items.length && <p>当前筛选及授权范围内没有事件。</p>}
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th>时间 / 来源</th>
              <th>操作者</th>
              <th>事件类型</th>
              <th>目标 / 范围</th>
              <th>结果</th>
              <th>详情</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => (
              <tr key={item.id}>
                <td>
                  {item.occurred_at
                    ? new Date(item.occurred_at * 1000).toLocaleString("zh-CN")
                    : "历史时间未知"}
                  <small>
                    {categories[item.category] ?? item.category}
                    {item.legacy && " · 历史记录"}
                  </small>
                </td>
                <td>
                  {actorLabel(item)}
                  <small>
                    {item.actor_id ??
                      (item.legacy ? "历史身份未记录" : "无操作员身份")}
                  </small>
                </td>
                <td>{item.operation}</td>
                <td>
                  {item.target ?? "未知"}
                  <small>{scopeLabel(item)}</small>
                </td>
                <td>{outcomes[item.outcome] ?? item.outcome}</td>
                <td>
                  <button onClick={() => void inspect(item.id)}>
                    查看事件
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <button
        disabled={loading || !cursor}
        onClick={() => {
          if (cursor) void load(query, cursor);
        }}
      >
        加载更早事件
      </button>
      {detail && (
        <DetailsDialog
          title={`事件 ${detail.id}`}
          onClose={() => {
            detailsSequence.current++;
            setDetail(null);
          }}
        >
          <dl>
            <dt>来源标识</dt>
            <dd>
              {detail.source} / {detail.source_id}
            </dd>
            <dt>事件类型</dt>
            <dd>{detail.operation}</dd>
            <dt>操作者</dt>
            <dd>
              {actorLabel(detail)} ·{" "}
              {detail.actor_id ??
                (detail.legacy ? "历史稳定身份未知" : "无操作员身份")}
            </dd>
            <dt>机房范围</dt>
            <dd>{scopeLabel(detail)}</dd>
            <dt>比赛</dt>
            <dd>{detail.contest_id ?? "未知 / 不适用"}</dd>
            <dt>请求编号</dt>
            <dd>{detail.request_id ?? "未知"}</dd>
            <dt>版本</dt>
            <dd>{detail.version ?? "未知"}</dd>
            <dt>结果</dt>
            <dd>{outcomes[detail.outcome] ?? detail.outcome}</dd>
          </dl>
          <div className="actions">
            {detail.request_id && (
              <button
                onClick={() => {
                  const request_id = detail.request_id!;
                  setFilters({ request_id });
                  setDetail(null);
                  void load(new URLSearchParams({ request_id }).toString());
                }}
              >
                查询关联事件
              </button>
            )}
            {detail.device_ids.map((id) => (
              <a
                key={id}
                href={`#/devices?device_id=${encodeURIComponent(id)}`}
              >
                查看设备 {id}
              </a>
            ))}
            {detail.contest_id && (
              <a
                href={`#/roster?contest_id=${encodeURIComponent(detail.contest_id)}`}
              >
                查看比赛名单
              </a>
            )}
            {detail.contest_id &&
              (detail.operation.startsWith("waiting_") ||
                detail.operation.startsWith("login_")) && (
                <a
                  href={`#/${detail.operation.startsWith("login_") ? "login" : "waiting"}?contest_id=${encodeURIComponent(detail.contest_id)}`}
                >
                  {detail.operation.startsWith("login_")
                    ? "查看比赛账户"
                    : "查看等待控制"}
                </a>
              )}
            {detail.operation === "publish" && (
              <a href="#/configuration">查看当前工具配置</a>
            )}
            {detail.operation === "operator_save" && (
              <a href="#/operators">查看操作员</a>
            )}
          </div>
          <h3>告警与事件处置</h3>
          <p>
            {detail.handling
              ? `最近处理人 ${detail.handling.actor_id} · ${new Date(detail.handling.updated_at * 1000).toLocaleString("zh-CN")}`
              : "尚无处置记录"}
          </p>
          <label>
            处理状态
            <select
              value={handling}
              disabled={disabled || saving}
              onChange={(e) => setHandling(e.target.value)}
            >
              <option value="open">待处理</option>
              <option value="acknowledged">已接手</option>
              <option value="resolved">已处理</option>
            </select>
          </label>
          <label>
            处置说明
            <textarea
              value={note}
              disabled={disabled || saving}
              maxLength={1000}
              onChange={(e) => setNote(e.target.value)}
            />
          </label>
          <button
            disabled={disabled || saving || !note.trim()}
            onClick={async () => {
              setSaving(true);
              setError("");
              try {
                await api(
                  "events/action",
                  {
                    request_id: crypto.randomUUID(),
                    event_id: Number(detail.id),
                    version: detail.handling?.version ?? 0,
                    status: handling,
                    note,
                  },
                  csrf,
                );
                await inspect(detail.id);
                await load(query);
              } catch (error) {
                setError(
                  error instanceof Error ? error.message : "处置保存失败",
                );
              } finally {
                setSaving(false);
              }
            }}
          >
            保存处置
          </button>
          <h3>持久结果</h3>
          <pre className="event-detail">
            {JSON.stringify(detail.detail, null, 2)}
          </pre>
        </DetailsDialog>
      )}
    </section>
  );
}
