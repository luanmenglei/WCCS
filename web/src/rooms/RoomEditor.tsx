import { useEffect, useRef, useState } from "react";
import type { z } from "zod";
import { api, type Mutation } from "../api";
import type { Snapshot } from "../contracts";
import {
  errors,
  layout,
  roomPreviewSchema,
  type Draft,
  type DraftSeat,
} from "../seating";
import { SeatChecks } from "../SeatChecks";
import { BatchIpEditor } from "./BatchIpEditor";
import { SeatGrid } from "./SeatGrid";

export function RoomEditor({
  initial,
  onReload,
  onDirty,
  ...props
}: {
  snapshot: Snapshot;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
  initial: Draft;
  onReload: () => void;
  onDirty: (dirty: boolean) => void;
}) {
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
  const [edited, setEdited] = useState(false);
  const [showUnchanged, setShowUnchanged] = useState(false);
  const dirty = edited || dimensionsPending || !!batch || !!text;
  useEffect(() => {
    onDirty(dirty);
  }, [dirty, onDirty]);
  const invalid = errors(draft);
  const disabled = props.disabled || busy;
  // 预览令牌和请求编号只对应当时的草稿；编辑后需要重新预览才能保存。
  function edit(next: Draft) {
    setEdited(true);
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
      {dirty && (
        <p role="status">
          有未保存的修改。在线状态来自已生效的布局；请先保存，再对新绑定发起检查。
        </p>
      )}
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
        <SeatGrid
          draft={draft}
          current={current}
          devices={props.snapshot.devices}
          zoom={zoom}
          invalid={invalid}
          selected={selected}
          setSelected={setSelected}
          edit={edit}
        />
        <BatchIpEditor
          draft={draft}
          batch={batch}
          text={text}
          setText={setText}
          attempt={attempt}
          onPreview={(seats) => {
            setBatch(seats);
            setPreview(null);
          }}
          onApply={(seats) => {
            edit({ ...draft, seats });
            setText("");
          }}
        />
        <div className="room-actions">
          <button
            className="primary"
            disabled={
              conflict ||
              dimensionsPending ||
              invalid.size > 0 ||
              !draft.name.trim() ||
              !!batch ||
              !!text
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
            <label>
              <input
                type="checkbox"
                checked={showUnchanged}
                onChange={(event) => setShowUnchanged(event.target.checked)}
              />
              显示未变化的位置
            </label>
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
              。其中{" "}
              {
                preview.changes.filter(
                  (change) =>
                    change.kind === "retained" &&
                    change.previous?.expected_ip !== change.expected_ip,
                ).length
              }{" "}
              个保留位置更换了 IP。移除位置将解除其 IP 与座位关联。
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
                  {preview.changes
                    .filter(
                      (change) =>
                        showUnchanged ||
                        change.kind !== "retained" ||
                        change.previous?.number !== change.number ||
                        change.previous.expected_ip !== change.expected_ip ||
                        change.previous.devices.join(",") !==
                          change.incoming_devices
                            .map((device) => device.id)
                            .join(","),
                    )
                    .map((change) => (
                      <tr key={`${change.row},${change.column}`}>
                        <td>
                          {change.row + 1} 行 {change.column + 1} 列
                        </td>
                        <td>
                          {
                            {
                              added: "新增",
                              retained: "保留",
                              removed: "移除",
                            }[change.kind]
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
                  setEdited(false);
                  setText("");
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
            disabled={disabled || dirty || conflict || !!preview}
            onMutate={props.onMutate}
          />
        )}
      </fieldset>
      {busy && <p role="status">正在处理…</p>}
    </div>
  );
}
