import { useState } from "react";
import {
  errors,
  fillIps,
  pasteIps,
  type Draft,
  type DraftSeat,
} from "../seating";

export function BatchIpEditor({
  draft,
  batch,
  text,
  setText,
  onPreview,
  onApply,
  attempt,
}: {
  draft: Draft;
  batch: DraftSeat[] | null;
  text: string;
  setText: (value: string) => void;
  onPreview: (seats: DraftSeat[] | null) => void;
  onApply: (seats: DraftSeat[]) => void;
  attempt: (action: () => void) => void;
}) {
  const [startIp, setStartIp] = useState("10.10.0.1");
  const [step, setStep] = useState(1);
  const [order, setOrder] = useState("number");
  const [pasteMode, setPasteMode] = useState("pairs");
  const issues = batch
    ? errors({ ...draft, seats: batch })
    : new Map<string, string>();
  return (
    <details className="batch-editor">
      <summary>批量绑定 IP</summary>
      <div className="room-controls">
        <label>
          起始 IPv4
          <input
            value={startIp}
            onChange={(event) => {
              setStartIp(event.target.value);
              onPreview(null);
            }}
          />
        </label>
        <label>
          步长
          <input
            type="number"
            min={1}
            value={step}
            onChange={(event) => {
              setStep(Number(event.target.value));
              onPreview(null);
            }}
          />
        </label>
        <label>
          分配顺序
          <select
            value={order}
            onChange={(event) => {
              setOrder(event.target.value);
              onPreview(null);
            }}
          >
            <option value="number">座位号递增</option>
            <option value="position">行优先，从左上开始</option>
          </select>
        </label>
        <button
          onClick={() =>
            attempt(() => {
              onPreview(fillIps(draft.seats, startIp, step, order));
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
          onChange={(event) => {
            setPasteMode(event.target.value);
            onPreview(null);
          }}
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
          onChange={(event) => {
            setText(event.target.value);
            onPreview(null);
          }}
          placeholder="从表格复制制表符分隔的单元格，空白 IP 表示清空"
        />
      </label>
      <button
        onClick={() =>
          attempt(() => {
            onPreview(pasteIps(draft, text, pasteMode));
          })
        }
      >
        预览粘贴映射
      </button>
      {batch && (
        <div>
          <h4>IP 映射预览</h4>
          <Mapping before={draft.seats} after={batch} issues={issues} />
          <button
            disabled={issues.size > 0}
            onClick={() => {
              onApply(batch);
            }}
          >
            应用映射到草稿
          </button>
          {issues.size > 0 && (
            <p role="alert" className="error">
              映射存在重复地址或编号错误，请修正输入后重新预览。
            </p>
          )}
        </div>
      )}
    </details>
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
