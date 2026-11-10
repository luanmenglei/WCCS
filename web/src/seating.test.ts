import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Rooms } from "./Rooms";
import { stateSchema } from "./contracts";
import {
  errors,
  fillIps,
  layout,
  newRoom,
  pasteIps,
  type Draft,
} from "./seating";

function room(): Draft {
  const draft = { ...newRoom(), rows: 2, columns: 3 };
  return { ...draft, seats: layout(draft) };
}

test("四个起点与按行、按列、蛇形均产生连续编号", () => {
  for (const origin of [
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
  ] as const) {
    for (const numbering of ["row", "column", "snake"] as const) {
      const draft = { ...room(), origin, numbering };
      draft.seats = layout(draft);
      assert.deepEqual(
        draft.seats.map((seat) => seat.number).sort((a, b) => a - b),
        [1, 2, 3, 4, 5, 6],
      );
      assert.equal(errors(draft).size, 0);
    }
  }
  assert.deepEqual(
    layout({ ...room(), numbering: "snake" }).map((seat) => seat.number),
    [1, 2, 3, 6, 5, 4],
  );
  assert.deepEqual(
    layout({ ...room(), numbering: "column" }).map((seat) => seat.number),
    [1, 3, 5, 2, 4, 6],
  );
});

test("重编号与尺寸变化保留物理位置 IP，自定义错误定位到所有重复格", () => {
  const draft = room();
  draft.seats[0].expected_ip = "10.0.0.5";
  const numbered = { ...draft, numbering: "column" as const, start_number: 20 };
  numbered.seats = layout(numbered);
  assert.equal(numbered.seats[0].expected_ip, "10.0.0.5");
  const resized = layout({ ...numbered, rows: 3, columns: 2 });
  assert.equal(resized[0].expected_ip, "10.0.0.5");
  assert.equal(resized.length, 6);
  const custom = { ...draft, numbering: "custom" as const };
  [custom.seats[0].number, custom.seats[5].number] = [6, 1];
  assert.equal(errors(custom).size, 0);
  custom.seats[1].number = 6;
  assert.equal(errors(custom).size, 2);
});

test("IPv4 顺序填充按编号或物理位置，拒绝溢出及非法步长", () => {
  const draft = room();
  [draft.seats[0].number, draft.seats[5].number] = [6, 1];
  assert.equal(
    fillIps(draft.seats, "10.0.0.250", 2, "number")[0].expected_ip,
    "10.0.1.4",
  );
  assert.equal(
    fillIps(draft.seats, "10.0.0.250", 2, "position")[0].expected_ip,
    "10.0.0.250",
  );
  assert.throws(() => fillIps(draft.seats, "255.255.255.254", 1, "number"));
  assert.throws(() => fillIps(draft.seats, "010.0.0.1", 1, "number"));
  assert.throws(() => fillIps(draft.seats, "10.0.0.1", 0, "number"));
});

test("两列表格和矩阵保留非目标位置，空白清空，校验重复、数量和地址", () => {
  const draft = room();
  draft.seats = fillIps(draft.seats, "10.0.0.1", 1, "number");
  const pairs = pasteIps(draft, "1\t10.1.2.9\n6\t\n", "pairs");
  assert.equal(pairs[0].expected_ip, "10.1.2.9");
  assert.equal(pairs[1].expected_ip, "10.0.0.2");
  assert.equal(pairs[5].expected_ip, null);
  const matrix = pasteIps(
    draft,
    "10.0.1.7\t\t10.0.1.99\n\t10.0.2.3\t",
    "matrix",
  );
  assert.equal(matrix[1].expected_ip, null);
  assert.equal(matrix[4].expected_ip, "10.0.2.3");
  assert.throws(() => pasteIps(draft, "1\t10.0.0.1\n1\t10.0.0.2", "pairs"));
  assert.throws(() => pasteIps(draft, "9\t10.0.0.1", "pairs"));
  assert.throws(() => pasteIps(draft, "1\t256.0.0.1", "pairs"));
  assert.throws(() => pasteIps(draft, "10.0.0.1\t10.0.0.2", "matrix"));
  const duplicate = pasteIps(draft, "1\t10.0.0.2", "pairs");
  assert.equal(errors({ ...draft, seats: duplicate }).size, 2);
});

test("机房入口提供创建、尺寸和未分配设备核对", () => {
  const snapshot = stateSchema.parse({
    protocol_version: 1,
    server_time: 0,
    version: 0,
    configuration: { schema_version: 1, config_version: "0", tools: [] },
    targets: [],
    catalog: [],
    devices: [],
    seating: {
      rooms: [],
      devices: [
        {
          id: "test",
          peer_ip: null,
          observed_ip: null,
          address_source: null,
          reported_ips: [],
          previous_ip: null,
          address_version: 0,
          needs_review: false,
          excluded: false,
          registered: false,
          online: false,
        },
      ],
    },
  });
  const html = renderToStaticMarkup(
    createElement(Rooms, {
      snapshot,
      csrf: "test",
      disabled: false,
      onMutate: async () => {},
    }),
  );
  assert.match(html, /创建机房/);
  assert.match(html, /未分配/);
  assert.match(html, /核对.*恢复匹配/);
});
