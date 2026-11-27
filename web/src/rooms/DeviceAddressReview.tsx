import { seatStatusLabels } from "../fleet";
import { useState } from "react";
import type { Snapshot } from "../contracts";
import type { Mutation } from "../api";
import type { Room, Seat } from "../seating";

export function DeviceAddressReview(props: {
  snapshot: Snapshot;
  contestId?: string;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const [exceptions, setExceptions] = useState(false);
  const [query, setQuery] = useState("");
  const search = query.trim().toLowerCase();
  const names = new Map(
    props.snapshot.devices.map((device) => [device.id, device.name]),
  );
  const roster = props.snapshot.roster.contests.find(
    (contest) => contest.id === props.contestId,
  );
  const participants = new Map<string, string>();
  for (const person of roster?.participants ?? []) {
    if (person.seat_id && !participants.has(person.seat_id))
      participants.set(person.seat_id, person.name);
  }
  // 一个设备可能占用多个待核对位置，保留全部匹配供管理员判断。
  const seatsByDevice = new Map<string, { room: Room; seat: Seat }[]>();
  for (const room of props.snapshot.seating.rooms) {
    for (const seat of room.seats) {
      for (const deviceId of new Set(seat.devices)) {
        const locations = seatsByDevice.get(deviceId) ?? [];
        locations.push({ room, seat });
        seatsByDevice.set(deviceId, locations);
      }
    }
  }
  return (
    <>
      <h3>原位换电脑 / 地址核对</h3>
      <p>
        选手不换座时，不要改名单。先找到物理座位并核对新旧电脑，再处理旧设备占用。选手要去另一座位，请到{" "}
        <a href="#/roster">名单与分配</a>。
      </p>
      <label>
        查找机房、座号、机器或 IP{" "}
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>
      <label>
        <input
          type="checkbox"
          checked={exceptions}
          onChange={(event) => setExceptions(event.target.checked)}
        />
        只看冲突、待核对或未匹配
      </label>
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
              const matches = seatsByDevice.get(device.id) ?? [];
              const machine = names.get(device.id) ?? device.id;
              const locations = matches
                .map(({ room, seat }) => {
                  const person = participants.get(seat.id);
                  return `${room.name} / ${seat.number}（${roster ? (person ?? "本场空座") : "未选比赛"}）`;
                })
                .join("、");
              if (
                exceptions &&
                matches.length &&
                !device.needs_review &&
                !matches.some(({ seat }) => seat.status === "conflict")
              )
                return null;
              if (
                ![
                  locations,
                  device.id,
                  machine,
                  device.observed_ip,
                  device.previous_ip,
                  ...device.reported_ips,
                ]
                  .join(" ")
                  .toLowerCase()
                  .includes(search)
              )
                return null;
              return (
                <tr key={device.id}>
                  <td>
                    {machine} · {device.id}
                    <br />
                    {matches.length
                      ? matches
                          .map(
                            ({ room, seat }) =>
                              `${room.name} / ${seat.number} · ${seatStatusLabels[seat.status]}`,
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
                      disabled={
                        props.disabled ||
                        (!device.excluded && !device.needs_review)
                      }
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
                      {device.excluded
                        ? "恢复参与匹配"
                        : device.needs_review
                          ? "确认新地址"
                          : "无需核对"}
                    </button>
                    <button
                      disabled={props.disabled || device.excluded}
                      onClick={() =>
                        window.confirm(
                          `排除 ${machine}（${device.observed_ip ?? "地址未知"}）在 ${locations || "当前未匹配座位"} 的占用？不会撤销凭据或移动选手。`,
                        ) &&
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
    </>
  );
}
