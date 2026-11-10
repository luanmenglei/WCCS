import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { stateSchema } from "./contracts";
import { checkStatus, matchesSearch, pageFromHash, seatRows } from "./fleet";
import { Seats } from "./Seats";
import { Devices } from "./Devices";

const snapshot = stateSchema.parse({
  protocol_version: 1,
  server_time: 0,
  version: 1,
  configuration: { schema_version: 1, config_version: "1", tools: [] },
  targets: ["device-a"],
  catalog: [],
  devices: [
    {
      id: "device-a",
      name: "合成设备",
      registered: true,
      online: true,
      last_seen: 0,
      desired_job: "job",
      desired_version: 1,
      expired: false,
      stale: false,
      result: null,
      received_at: null,
    },
  ],
  seating: {
    devices: [],
    rooms: [
      {
        id: "room-a",
        name: "合成机房",
        rows: 1,
        columns: 2,
        start_number: 10,
        numbering: "custom",
        origin: "top-left",
        version: 1,
        seats: [
          {
            id: "seat-a",
            room_id: "room-a",
            row: 0,
            column: 0,
            number: 11,
            expected_ip: "192.0.2.11",
            status: "online",
            devices: ["device-a"],
          },
          {
            id: "seat-b",
            room_id: "room-a",
            row: 0,
            column: 1,
            number: 10,
            expected_ip: null,
            status: "unbound",
            devices: [],
          },
        ],
      },
    ],
  },
});

test("新页面路由兼容原书签，未知路径回到座位页", () => {
  assert.equal(pageFromHash("#/devices"), "devices");
  assert.equal(pageFromHash("#rooms"), "rooms");
  assert.equal(pageFromHash("#/targets"), "targets");
  assert.equal(pageFromHash("#invalid"), "seats");
  assert.equal(pageFromHash("#__proto__"), "seats");
});

test("跨机房座位按物理位置展开，搜索支持中文、数字和大小写", () => {
  assert.deepEqual(
    seatRows(snapshot).map(({ seat }) => seat.number),
    [11, 10],
  );
  assert.equal(
    matchesSearch(["合成机房", "Device-A", 11, null], "device-a"),
    true,
  );
  assert.equal(matchesSearch(["合成机房", 11], "机房"), true);
  assert.equal(matchesSearch([11], "11"), true);
  assert.equal(matchesSearch(["设备"], "未知"), false);
});

test("工具状态保留未注册、过期和等待回传", () => {
  const device = snapshot.devices[0];
  assert.equal(checkStatus(device), "等待回传");
  assert.equal(checkStatus({ ...device, registered: false }), "待注册");
  assert.equal(checkStatus({ ...device, expired: true }), "任务已过期");
});

test("座位图展示真实绑定覆盖率、物理编号与选择状态", () => {
  const html = renderToStaticMarkup(
    createElement(Seats, {
      snapshot,
      csrf: "test",
      disabled: false,
      onMutate: async () => {},
      selected: ["seat-a"],
      onSelect: () => {},
    }),
  );
  assert.match(html, /绑定覆盖率/);
  assert.match(html, /value="50"/);
  assert.match(html, /合成机房 座位 11 详情/);
  assert.match(html, /checked/);
  assert.match(html, /192\.0\.2\.11/);
});

test("设备清单展示期望与实际状态，空匹配保留未分配", () => {
  const html = renderToStaticMarkup(
    createElement(Devices, {
      snapshot,
      disabled: true,
      onCreate: async () => {},
      onRecheck: async () => {},
    }),
  );
  assert.match(html, /设备筛选/);
  assert.match(html, /合成机房 \/ 11/);
  assert.match(html, /等待回传/);
  assert.match(html, /查看详情/);
  assert.match(html, /button[^>]*disabled[^>]*>新增设备/);
});
