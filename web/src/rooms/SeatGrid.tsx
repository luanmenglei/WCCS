import { seatStatusLabels } from "../fleet";
import { checkLabel, type Snapshot } from "../contracts";
import type { Room, Draft } from "../seating";

export function SeatGrid({
  draft,
  current,
  devices: liveDevices,
  zoom,
  invalid,
  selected,
  setSelected,
  edit,
}: {
  draft: Draft;
  current?: Room;
  devices: Snapshot["devices"];
  zoom: string;
  invalid: Map<string, string>;
  selected: string[];
  setSelected: (ids: string[]) => void;
  edit: (next: Draft) => void;
}) {
  const liveSeats = new Map(
    current?.seats.map((seat) => [`${seat.row},${seat.column}`, seat]),
  );
  const devices = new Map(liveDevices.map((device) => [device.id, device]));
  return (
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
                      {live && live.expected_ip !== seat.expected_ip && (
                        <small>
                          生效 IP：{live.expected_ip ?? "未配置"} → 草稿：
                          {seat.expected_ip ?? "清空"}
                        </small>
                      )}
                      {invalid.has(key) && (
                        <span role="alert" className="error">
                          {invalid.get(key)}
                        </span>
                      )}
                      <span
                        className={`seat-state ${live?.status ?? "unbound"}`}
                      >
                        {live ? seatStatusLabels[live.status] : "新增位置"}
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
  );
}
