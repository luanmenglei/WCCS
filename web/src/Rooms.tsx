import { useRef, useState } from "react";
import { SeatChecks } from "./SeatChecks";
import { api } from "./api";
import { checkLabel, type Snapshot } from "./contracts";
import {
  errors,
  fillIps,
  layout,
  newRoom,
  pasteIps,
  roomPreviewSchema,
  type Draft,
  type DraftSeat,
} from "./seating";
import type { z } from "zod";

type Props = {
  snapshot: Snapshot;
  csrf: string;
  disabled: boolean;
  onMutate: (
    path: string,
    body: Record<string, unknown>,
    message: string,
  ) => Promise<void>;
};
const statuses: Record<string, string> = {
  online: "在线",
  offline: "离线",
  unbound: "未绑定",
  conflict: "冲突",
  ready: "可检查",
  unregistered: "未注册 / 未匹配，跳过",
  outside_configuration: "不在全局配置目标内，跳过",
};

export function Rooms(props: Props) {
  const [draft, setDraft] = useState<Draft>(() => {
    const room = newRoom();
    return { ...room, seats: layout(room) };
  });
  const [key, setKey] = useState(0);
  const [open, setOpen] = useState(false);
  function load(room: Draft) {
    setDraft(room);
    setKey(key + 1);
    setOpen(true);
  }
  return (
    <section id="rooms" className="panel rooms-panel">
      <div className="section-header">
        <div>
          <h2>机房与座位</h2>
          <p>按物理位置维护座位，认证设备通过来源 IP 匹配。</p>
        </div>
        <button
          disabled={props.disabled}
          onClick={() => {
            const room = newRoom();
            load({ ...room, seats: layout(room) });
          }}
        >
          创建机房
        </button>
      </div>
      <div className="room-tabs">
        {props.snapshot.seating.rooms.map((room) => (
          <button
            key={room.id}
            aria-pressed={open && draft.id === room.id}
            onClick={() => load(room)}
          >
            {room.name} · {room.rows} × {room.columns} · v{room.version}
          </button>
        ))}
      </div>
      {open && (
        <RoomEditor
          key={key}
          {...props}
          initial={draft}
          onReload={() => {
            const room = props.snapshot.seating.rooms.find(
              (room) => room.id === draft.id,
            );
            if (room) load(room);
          }}
        />
      )}
      <h3>地址与设备核对</h3>
      <p>
        来源地址由服务端连接或显式信任代理提供。客户端报告地址仅用于核对。排除占用后可恢复参与匹配。
      </p>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th>设备 / 关联</th>
              <th>预期 IP</th>
              <th>服务端观察 / 连接对端</th>
              <th>客户端报告</th>
              <th>核对状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {props.snapshot.seating.devices.map((device) => {
              const matches = props.snapshot.seating.rooms.flatMap((room) =>
                room.seats
                  .filter((seat) => seat.devices.includes(device.id))
                  .map((seat) => ({ room, seat })),
              );
              return (
                <tr key={device.id}>
                  <td>
                    {device.id}
                    <br />
                    {matches.length
                      ? matches
                          .map(
                            ({ room, seat }) =>
                              `${room.name} / ${seat.number} · ${statuses[seat.status]}`,
                          )
                          .join("、")
                      : "未分配"}
                  </td>
                  <td>
                    {matches.map(({ seat }) => seat.expected_ip).join("、") ||
                      "—"}
                  </td>
                  <td>
                    {device.observed_ip ?? "未知"}
                    <br />
                    <small>
                      {device.address_source ?? "尚未认证心跳"} · 对端{" "}
                      {device.peer_ip ?? "—"}
                    </small>
                  </td>
                  <td>{device.reported_ips.join("、") || "未报告"}</td>
                  <td>
                    {device.excluded
                      ? "已排除"
                      : device.needs_review
                        ? `地址变化待核对（此前 ${device.previous_ip ?? "未知"}）`
                        : device.online
                          ? "在线"
                          : "离线"}
                  </td>
                  <td>
                    <button
                      disabled={props.disabled}
                      onClick={() =>
                        void props
                          .onMutate(
                            "seating/review",
                            {
                              device_id: device.id,
                              base_version: device.address_version,
                              exclude: false,
                            },
                            "已核对当前地址，重新计算座位匹配。",
                          )
                          .catch(() => {})
                      }
                    >
                      核对 / 恢复匹配
                    </button>
                    <button
                      disabled={props.disabled || device.excluded}
                      onClick={() =>
                        void props
                          .onMutate(
                            "seating/review",
                            {
                              device_id: device.id,
                              base_version: device.address_version,
                              exclude: true,
                            },
                            "已排除此设备的座位占用。",
                          )
                          .catch(() => {})
                      }
                    >
                      排除占用
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function RoomEditor({
  initial,
  onReload,
  ...props
}: Props & { initial: Draft; onReload: () => void }) {
  const [draft, setDraft] = useState(initial);
  const [dimensions, setDimensions] = useState({
    rows: initial.rows,
    columns: initial.columns,
  });
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<z.infer<
    typeof roomPreviewSchema
  > | null>(null);
  const [batch, setBatch] = useState<DraftSeat[] | null>(null);
  const [startIp, setStartIp] = useState("10.10.0.1");
  const [step, setStep] = useState(1);
  const [order, setOrder] = useState("number");
  const [pasteMode, setPasteMode] = useState("pairs");
  const [text, setText] = useState("");
  const [selected, setSelected] = useState<string[]>([]);

  const [zoom, setZoom] = useState("normal");
  const requestId = useRef(crypto.randomUUID());
  const current = props.snapshot.seating.rooms.find(
    (room) => room.id === draft.id,
  );
  const conflict = (current?.version ?? 0) !== draft.version;
  const dimensionsPending =
    dimensions.rows !== draft.rows || dimensions.columns !== draft.columns;
  const invalid = errors(draft);
  const disabled = props.disabled || busy;
  const liveSeats = new Map(
    current?.seats.map((seat) => [`${seat.row},${seat.column}`, seat]),
  );
  const devices = new Map(
    props.snapshot.devices.map((device) => [device.id, device]),
  );
  function edit(next: Draft) {
    setDraft(next);
    setPreview(null);
    setBatch(null);
    requestId.current = crypto.randomUUID();
  }
  function attempt(action: () => void) {
    setError("");
    try {
      action();
    } catch (error) {
      setError(error instanceof Error ? error.message : "操作失败");
    }
  }
  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (error) {
      setError(error instanceof Error ? error.message : "操作失败");
    } finally {
      setBusy(false);
    }
  }
  function body() {
    const { version, seats, ...fields } = draft;
    return {
      ...fields,
      base_version: version,
      seats: seats.map(({ row, column, number, expected_ip }) => ({
        row,
        column,
        number,
        expected_ip,
      })),
      request_id: requestId.current,
    };
  }
  return (
    <div className="room-editor">
      <div className="section-header">
        <h3>
          {draft.name || "新机房"} · 编辑基准 v{draft.version}
        </h3>
        <button disabled={disabled || !current} onClick={onReload}>
          重新加载机房
        </button>
      </div>
      {conflict && (
        <p role="alert" className="error">
          机房已更新到 v{current?.version}，草稿已保留。请重新加载后编辑。
        </p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {dimensionsPending && (
        <p role="status">行列尺寸已修改，请先应用尺寸到草稿再预览保存。</p>
      )}
      <fieldset disabled={disabled}>
        <div className="room-controls">
          <label>
            机房名称
            <input
              value={draft.name}
              maxLength={100}
              onChange={(event) => edit({ ...draft, name: event.target.value })}
            />
          </label>
          <label>
            行数
            <input
              type="number"
              min={1}
              max={50}
              value={dimensions.rows}
              onChange={(event) =>
                setDimensions({
                  ...dimensions,
                  rows: Number(event.target.value),
                })
              }
            />
          </label>
          <label>
            列数
            <input
              type="number"
              min={1}
              max={50}
              value={dimensions.columns}
              onChange={(event) =>
                setDimensions({
                  ...dimensions,
                  columns: Number(event.target.value),
                })
              }
            />
          </label>
          <button
            onClick={() =>
              attempt(() => {
                const next = { ...draft, ...dimensions };
                edit({ ...next, seats: layout(next) });
              })
            }
          >
            应用尺寸到草稿
          </button>
        </div>
        <div className="room-controls">
          <label>
            起始号
            <input
              type="number"
              min={0}
              max={1000000}
              value={draft.start_number}
              onChange={(event) =>
                edit({ ...draft, start_number: Number(event.target.value) })
              }
            />
          </label>
          <label>
            编号规则
            <select
              value={draft.numbering}
              onChange={(event) =>
                edit({
                  ...draft,
                  numbering: event.target.value as Draft["numbering"],
                })
              }
            >
              <option value="row">按行</option>
              <option value="column">按列</option>
              <option value="snake">蛇形</option>
              <option value="custom">自定义排列</option>
            </select>
          </label>
          <label>
            编号起点
            <select
              value={draft.origin}
              onChange={(event) =>
                edit({
                  ...draft,
                  origin: event.target.value as Draft["origin"],
                })
              }
            >
              <option value="top-left">左上</option>
              <option value="top-right">右上</option>
              <option value="bottom-left">左下</option>
              <option value="bottom-right">右下</option>
            </select>
          </label>
          <button
            onClick={() =>
              attempt(() => edit({ ...draft, seats: layout(draft) }))
            }
          >
            应用编号规则
          </button>
          <label>
            布局缩放
            <select
              value={zoom}
              onChange={(event) => setZoom(event.target.value)}
            >
              <option value="compact">紧凑</option>
              <option value="normal">标准</option>
              <option value="large">放大</option>
            </select>
          </label>
        </div>
        <p>
          编号范围 {draft.start_number}～
          {draft.start_number + draft.rows * draft.columns - 1}，共{" "}
          {draft.seats.length} 个位置。逐格编辑编号会切换到自定义排列。
        </p>
        <div className={`table-scroll seat-layout ${zoom}`}>
          <table>
            <thead>
              <tr>
                <th>行 / 列</th>
                {Array.from({ length: draft.columns }, (_, column) => (
                  <th key={column}>第 {column + 1} 列</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {Array.from({ length: draft.rows }, (_, row) => (
                <tr key={row}>
                  <th>第 {row + 1} 行</th>
                  {draft.seats
                    .filter((seat) => seat.row === row)
                    .map((seat) => {
                      const key = `${seat.row},${seat.column}`;
                      const live = liveSeats.get(key);
                      const device =
                        live?.devices.length === 1
                          ? devices.get(live.devices[0])
                          : undefined;
                      return (
                        <td
                          key={key}
                          className={invalid.has(key) ? "seat-invalid" : ""}
                        >
                          <label>
                            座位号
                            <input
                              aria-label={`第${row + 1}行第${seat.column + 1}列座位号`}
                              type="number"
                              value={seat.number}
                              onChange={(event) =>
                                edit({
                                  ...draft,
                                  numbering: "custom",
                                  seats: draft.seats.map((item) =>
                                    item === seat
                                      ? {
                                          ...item,
                                          number: Number(event.target.value),
                                        }
                                      : item,
                                  ),
                                })
                              }
                            />
                          </label>
                          <label>
                            预期 IP
                            <input
                              aria-label={`第${row + 1}行第${seat.column + 1}列IP`}
                              placeholder="未配置"
                              value={seat.expected_ip ?? ""}
                              onChange={(event) =>
                                edit({
                                  ...draft,
                                  seats: draft.seats.map((item) =>
                                    item === seat
                                      ? {
                                          ...item,
                                          expected_ip:
                                            event.target.value.trim() || null,
                                        }
                                      : item,
                                  ),
                                })
                              }
                            />
                          </label>
                          {invalid.has(key) && (
                            <span role="alert" className="error">
                              {invalid.get(key)}
                            </span>
                          )}
                          <span
                            className={`seat-state ${live?.status ?? "unbound"}`}
                          >
                            {live ? statuses[live.status] : "新增位置"}
                          </span>
                          {live && (
                            <label className="seat-select">
                              <input
                                type="checkbox"
                                checked={selected.includes(live.id)}
                                onChange={(event) => {
                                  setSelected(
                                    event.target.checked
                                      ? [...selected, live.id]
                                      : selected.filter((id) => id !== live.id),
                                  );
                                }}
                              />
                              选择检查
                            </label>
                          )}
                          {live?.devices.map((id) => (
                            <small key={id}>{id}</small>
                          ))}
                          {device && (
                            <small>
                              期望 v{device.desired_version ?? "—"} / 实际 v
                              {device.result?.config_version ?? "—"}
                              {device.stale ? " · 结果过期" : ""}
                              {device.expired ? " · 任务过期" : ""}
                            </small>
                          )}
                          {device?.result?.tools.map(({ tool, check }) => (
                            <small key={tool.id}>
                              {tool.name} · {checkLabel(check)}
                            </small>
                          ))}
                        </td>
                      );
                    })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <details className="batch-editor">
          <summary>批量绑定 IP</summary>
          <div className="room-controls">
            <label>
              起始 IPv4
              <input
                value={startIp}
                onChange={(event) => setStartIp(event.target.value)}
              />
            </label>
            <label>
              步长
              <input
                type="number"
                min={1}
                value={step}
                onChange={(event) => setStep(Number(event.target.value))}
              />
            </label>
            <label>
              分配顺序
              <select
                value={order}
                onChange={(event) => setOrder(event.target.value)}
              >
                <option value="number">座位号递增</option>
                <option value="position">行优先，从左上开始</option>
              </select>
            </label>
            <button
              onClick={() =>
                attempt(() => {
                  setBatch(fillIps(draft.seats, startIp, step, order));
                  setPreview(null);
                })
              }
            >
              预览顺序填充
            </button>
          </div>
          <label>
            粘贴格式
            <select
              value={pasteMode}
              onChange={(event) => setPasteMode(event.target.value)}
            >
              <option value="pairs">座位号 / IP 两列</option>
              <option value="matrix">按位置的 IP 矩阵</option>
            </select>
          </label>
          <label>
            表格内容
            <textarea
              rows={5}
              value={text}
              onChange={(event) => setText(event.target.value)}
              placeholder="从表格复制制表符分隔的单元格，空白 IP 表示清空"
            />
          </label>
          <button
            onClick={() =>
              attempt(() => {
                setBatch(pasteIps(draft, text, pasteMode));
                setPreview(null);
              })
            }
          >
            预览粘贴映射
          </button>
          {batch && (
            <div>
              <h4>IP 映射预览</h4>
              <Mapping
                before={draft.seats}
                after={batch}
                issues={errors({ ...draft, seats: batch })}
              />
              <button
                disabled={errors({ ...draft, seats: batch }).size > 0}
                onClick={() => edit({ ...draft, seats: batch })}
              >
                应用映射到草稿
              </button>
              {errors({ ...draft, seats: batch }).size > 0 && (
                <p role="alert" className="error">
                  映射存在重复地址或编号错误，请修正输入后重新预览。
                </p>
              )}
            </div>
          )}
        </details>
        <div className="room-actions">
          <button
            className="primary"
            disabled={
              conflict ||
              dimensionsPending ||
              invalid.size > 0 ||
              !draft.name.trim() ||
              !!batch
            }
            onClick={() =>
              void run(async () =>
                setPreview(
                  roomPreviewSchema.parse(
                    await api("rooms/preview", body(), props.csrf),
                  ),
                ),
              )
            }
          >
            预览保存影响
          </button>
        </div>
        {preview && (
          <div className="save-preview">
            <h4>保存影响预览</h4>
            <p>
              新增{" "}
              {
                preview.changes.filter((change) => change.kind === "added")
                  .length
              }
              ，保留{" "}
              {
                preview.changes.filter((change) => change.kind === "retained")
                  .length
              }
              ，移除{" "}
              {
                preview.changes.filter((change) => change.kind === "removed")
                  .length
              }
              。移除位置将解除其 IP 与座位关联。
            </p>
            <div className="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>位置</th>
                    <th>变化</th>
                    <th>编号</th>
                    <th>IP 变化</th>
                    <th>原关联设备 → 新匹配设备</th>
                  </tr>
                </thead>
                <tbody>
                  {preview.changes.map((change) => (
                    <tr key={`${change.row},${change.column}`}>
                      <td>
                        {change.row + 1} 行 {change.column + 1} 列
                      </td>
                      <td>
                        {
                          { added: "新增", retained: "保留", removed: "移除" }[
                            change.kind
                          ]
                        }
                      </td>
                      <td>
                        {change.previous?.number ?? "—"} →{" "}
                        {change.kind === "removed" ? "移除" : change.number}
                      </td>
                      <td>
                        {change.previous?.expected_ip ?? "未配置"} →{" "}
                        {change.expected_ip ?? "清空 / 未配置"}
                      </td>
                      <td>
                        {change.previous?.devices.join("、") || "—"} →{" "}
                        {change.incoming_devices
                          .map(
                            (device) =>
                              `${device.id}${device.needs_review ? "（待核对）" : ""}`,
                          )
                          .join("、") || "—"}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <button
              className="primary"
              disabled={conflict || dimensionsPending}
              onClick={() =>
                void run(async () => {
                  await props.onMutate(
                    "rooms/save",
                    { ...body(), preview_token: preview.preview_token },
                    "机房与 IP 已保存。",
                  );
                  edit({ ...draft, version: draft.version + 1 });
                })
              }
            >
              确认保存机房
            </button>
          </div>
        )}
        {current && (
          <div className="room-actions">
            <button
              onClick={() => {
                setSelected(current.seats.map((seat) => seat.id));
              }}
            >
              选择整间机房
            </button>
            <button
              onClick={() => {
                setSelected([]);
              }}
            >
              清除选择
            </button>
          </div>
        )}
        {current && (
          <SeatChecks
            snapshot={props.snapshot}
            selected={selected}
            csrf={props.csrf}
            disabled={disabled || conflict || !!preview}
            onMutate={props.onMutate}
          />
        )}
      </fieldset>
      {busy && <p role="status">正在处理…</p>}
    </div>
  );
}

function Mapping({
  before,
  after,
  issues,
}: {
  before: DraftSeat[];
  after: DraftSeat[];
  issues: Map<string, string>;
}) {
  const previous = new Map(
    before.map((seat) => [`${seat.row},${seat.column}`, seat.expected_ip]),
  );
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th>位置 / 座位号</th>
            <th>原 IP</th>
            <th>新 IP</th>
            <th>操作 / 校验</th>
          </tr>
        </thead>
        <tbody>
          {after.map((seat) => {
            const key = `${seat.row},${seat.column}`,
              old = previous.get(key);
            return (
              <tr key={key}>
                <td>
                  {seat.row + 1} 行 {seat.column + 1} 列 / {seat.number}
                </td>
                <td>{old ?? "未配置"}</td>
                <td>{seat.expected_ip ?? "清空"}</td>
                <td>
                  {issues.has(key) && (
                    <span className="error">{issues.get(key)} · </span>
                  )}
                  {old === seat.expected_ip
                    ? "保留"
                    : seat.expected_ip === null
                      ? "清空"
                      : old
                        ? "覆盖"
                        : "填入"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
