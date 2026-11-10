import { participantLabel, type Contest } from "./roster-model";
import { useState } from "react";
import { checkLabel, type Snapshot } from "./contracts";
import {
  checkStatus,
  matchesSearch,
  seatRows,
  seatStatusLabels,
} from "./fleet";
import { DetailsDialog } from "./DetailsDialog";
import { SeatChecks, type Mutation } from "./SeatChecks";

type Props = {
  contest?: Contest;
  snapshot: Snapshot;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
  targetMode?: boolean;
  selected: string[];
  onSelect: (ids: string[]) => void;
};

export function Seats({
  contest,
  snapshot,
  csrf,
  disabled,
  onMutate,
  targetMode = false,
  selected,
  onSelect,
}: Props) {
  const [roomId, setRoomId] = useState("");
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [view, setView] = useState(targetMode ? "list" : "map");
  const [detailId, setDetailId] = useState<string | null>(null);
  const all = seatRows(snapshot);
  const scoped = all.filter(({ room }) => !roomId || room.id === roomId);
  const visible = scoped.filter(
    ({ room, seat }) =>
      (filter === "all" ||
        seat.status === filter ||
        (filter === "bound" && ["online", "offline"].includes(seat.status))) &&
      matchesSearch(
        [
          room.name,
          seat.number,
          seat.expected_ip,
          participantLabel(contest, seat.id),
          ...seat.devices,
        ],
        search,
      ),
  );
  const visibleIds = new Set(visible.map(({ seat }) => seat.id));
  const selection = selected.filter((id) =>
    all.some(({ seat }) => seat.id === id),
  );
  const bound = scoped.filter(
    ({ seat }) => seat.status === "online" || seat.status === "offline",
  ).length;
  const coverage = scoped.length
    ? Math.round((bound / scoped.length) * 100)
    : 0;
  const detail = all.find(({ seat }) => seat.id === detailId);
  function toggle(id: string, checked: boolean) {
    onSelect(
      checked
        ? [...selected.filter((value) => value !== id), id]
        : selected.filter((value) => value !== id),
    );
  }

  return (
    <div className="fleet-page">
      <div className="stat-grid">
        <div className="stat-tile">
          <span>座位总数</span>
          <strong>{scoped.length}</strong>
          <small>{snapshot.seating.rooms.length} 间机房</small>
        </div>
        <div className="stat-tile good">
          <span>已绑定</span>
          <strong>{bound}</strong>
          <small>唯一匹配设备</small>
        </div>
        <div className="stat-tile attention">
          <span>需要绑定</span>
          <strong>
            {scoped.filter(({ seat }) => seat.status === "unbound").length}
          </strong>
          <small>等待设备匹配</small>
        </div>
        <div className="stat-tile danger">
          <span>冲突</span>
          <strong>
            {scoped.filter(({ seat }) => seat.status === "conflict").length}
          </strong>
          <small>地址或占用待核对</small>
        </div>
        <div className="stat-tile coverage">
          <span>绑定覆盖率</span>
          <strong>{coverage}%</strong>
          <progress aria-label="绑定覆盖率" value={coverage} max={100} />
        </div>
      </div>
      <div className="filter-bar">
        <label className="search-field">
          搜索座位
          <input
            type="search"
            placeholder="机房、座位号、IP 或设备 ID"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
        <label>
          机房
          <select
            value={roomId}
            onChange={(event) => setRoomId(event.target.value)}
          >
            <option value="">全部机房</option>
            {snapshot.seating.rooms.map((room) => (
              <option key={room.id} value={room.id}>
                {room.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          座位状态
          <select
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
          >
            <option value="all">全部状态</option>
            <option value="bound">已绑定</option>
            {Object.entries(seatStatusLabels).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <span className="result-count">
          {visible.length} / {scoped.length} 个座位
        </span>
        <div className="view-switch" aria-label="座位视图">
          <button aria-pressed={view === "map"} onClick={() => setView("map")}>
            座位图
          </button>
          <button
            aria-pressed={view === "list"}
            onClick={() => setView("list")}
          >
            列表
          </button>
        </div>
      </div>
      <div className="selection-bar">
        <span>已选择 {selection.length} 个座位</span>
        <button
          disabled={!visible.length}
          onClick={() =>
            onSelect([
              ...new Set([...selection, ...visible.map(({ seat }) => seat.id)]),
            ])
          }
        >
          选择筛选结果
        </button>
        <button disabled={!selection.length} onClick={() => onSelect([])}>
          清除选择
        </button>
      </div>
      {visible.length === 0 ? (
        <div className="panel empty">
          {all.length ? (
            "没有符合筛选条件的座位。调整搜索或状态筛选后重试。"
          ) : (
            <>
              尚无机房。前往 <a href="#/rooms">机房设置</a> 创建布局并配置 IP。
            </>
          )}
        </div>
      ) : view === "list" ? (
        <div className="panel table-scroll fleet-table">
          <table>
            <thead>
              <tr>
                <th>选择</th>
                <th>机房 / 座位</th>
                <th>选手</th>
                <th>物理位置</th>
                <th>预期 IP</th>
                <th>状态</th>
                <th>匹配设备</th>
                <th>详情</th>
              </tr>
            </thead>
            <tbody>
              {visible.map(({ room, seat }) => (
                <tr key={seat.id} className={`state-row ${seat.status}`}>
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`选择 ${room.name} 座位 ${seat.number}`}
                      checked={selection.includes(seat.id)}
                      onChange={(event) =>
                        toggle(seat.id, event.target.checked)
                      }
                    />
                  </td>
                  <td>
                    {room.name}
                    <strong className="seat-number">{seat.number}</strong>
                  </td>
                  <td>
                    {contest
                      ? participantLabel(contest, seat.id)
                      : "未选择比赛"}
                  </td>
                  <td>
                    {seat.row + 1} 行 {seat.column + 1} 列
                  </td>
                  <td className="mono">{seat.expected_ip ?? "未配置"}</td>
                  <td>{seatStatusLabels[seat.status]}</td>
                  <td className="mono">{seat.devices.join("、") || "—"}</td>
                  <td>
                    <button
                      className="quiet"
                      onClick={() => setDetailId(seat.id)}
                    >
                      查看详情
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="room-maps">
          {snapshot.seating.rooms
            .filter((room) => visible.some((item) => item.room.id === room.id))
            .map((room) => (
              <section
                className="panel room-map"
                key={room.id}
                aria-label={`${room.name} 座位图`}
              >
                <header className="section-header">
                  <h2>{room.name}</h2>
                  <span>
                    {room.rows} 行 × {room.columns} 列
                  </span>
                </header>
                <div className="table-scroll">
                  <table className="map-table">
                    <thead>
                      <tr>
                        <th>行 / 列</th>
                        {Array.from({ length: room.columns }, (_, column) => (
                          <th key={column}>{column + 1}</th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {Array.from({ length: room.rows }, (_, row) => (
                        <tr key={row}>
                          <th>{row + 1}</th>
                          {room.seats
                            .filter((seat) => seat.row === row)
                            .map((seat) => (
                              <td key={seat.id}>
                                {visibleIds.has(seat.id) ? (
                                  <div className={`seat-card ${seat.status}`}>
                                    <button
                                      aria-label={`${room.name} 座位 ${seat.number} 详情`}
                                      onClick={() => setDetailId(seat.id)}
                                    >
                                      <strong>{seat.number}</strong>
                                      <small>
                                        {contest
                                          ? participantLabel(contest, seat.id)
                                          : "未选择比赛"}
                                      </small>
                                      <span>
                                        {seatStatusLabels[seat.status]}
                                      </span>
                                      <small className="mono">
                                        {seat.expected_ip ?? "IP 未配置"}
                                      </small>
                                    </button>
                                    <label>
                                      <input
                                        type="checkbox"
                                        checked={selection.includes(seat.id)}
                                        onChange={(event) =>
                                          toggle(seat.id, event.target.checked)
                                        }
                                      />
                                      选择座位 {seat.number}
                                    </label>
                                  </div>
                                ) : (
                                  <div
                                    className="filtered-seat"
                                    aria-label="已被筛选隐藏的座位"
                                  />
                                )}
                              </td>
                            ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </section>
            ))}
        </div>
      )}
      <SeatChecks
        snapshot={snapshot}
        selected={selection}
        csrf={csrf}
        disabled={disabled}
        onMutate={onMutate}
      />
      {detail && (
        <DetailsDialog
          title={`${detail.room.name} · 座位 ${detail.seat.number}`}
          onClose={() => setDetailId(null)}
        >
          <dl className="facts">
            <dt>选手</dt>
            <dd>
              {contest
                ? participantLabel(contest, detail.seat.id)
                : "未选择比赛"}
            </dd>
            <dt>位置</dt>
            <dd>
              {detail.seat.row + 1} 行 {detail.seat.column + 1} 列
            </dd>
            <dt>预期 IP</dt>
            <dd className="mono">{detail.seat.expected_ip ?? "未配置"}</dd>
            <dt>绑定状态</dt>
            <dd>{seatStatusLabels[detail.seat.status]}</dd>
            <dt>座位 ID</dt>
            <dd className="mono">{detail.seat.id}</dd>
          </dl>
          {!detail.seat.devices.length && (
            <p className="empty">等待已认证设备从预期 IP 上线。</p>
          )}
          {detail.seat.devices.map((id) => {
            const device = snapshot.devices.find((device) => device.id === id);
            const address = snapshot.seating.devices.find(
              (device) => device.id === id,
            );
            return (
              <section className="detail-section" key={id}>
                <h3>{device?.name ?? id}</h3>
                <p className="mono">{id}</p>
                <dl className="facts">
                  <dt>服务端观察</dt>
                  <dd>{address?.observed_ip ?? "未知"}</dd>
                  <dt>来源</dt>
                  <dd>{address?.address_source ?? "未知"}</dd>
                  <dt>客户端报告</dt>
                  <dd>{address?.reported_ips.join("、") || "未报告"}</dd>
                  <dt>工具检查</dt>
                  <dd>{device ? checkStatus(device) : "尚无结果"}</dd>
                </dl>
                {device?.result?.tools.map(({ tool, check }) => (
                  <p key={tool.id}>
                    {tool.name} · {checkLabel(check)}
                  </p>
                ))}
              </section>
            );
          })}
          <a href="#/rooms" onClick={() => setDetailId(null)}>
            编辑机房与 IP
          </a>
        </DetailsDialog>
      )}
    </div>
  );
}
