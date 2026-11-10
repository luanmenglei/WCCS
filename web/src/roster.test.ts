import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { stateSchema } from "./contracts";
import { Roster } from "./Roster";
import { Seats } from "./Seats";
import {
  participantLabel,
  rosterPreviewSchema,
  contestSchema,
  sampleRoster,
} from "./roster-model";

const contest = contestSchema.parse({
  id: "first",
  name: "合成第一场",
  version: 2,
  participants: [
    {
      id: "person",
      code: "001",
      name: "合成甲",
      school: "合成学校",
      seat_id: "seat",
    },
  ],
});
const snapshot = stateSchema.parse({
  protocol_version: 1,
  server_time: 0,
  version: 0,
  registration_window: { version: 1, closes_at: 0, open: false },
  configuration: { schema_version: 1, config_version: "0", tools: [] },
  targets: [],
  catalog: [],
  devices: [],
  roster: { version: 1, contests: [contest] },
  seating: {
    devices: [],
    rooms: [
      {
        id: "room",
        name: "合成机房",
        rows: 1,
        columns: 1,
        start_number: 1,
        numbering: "row",
        origin: "top-left",
        version: 1,
        seats: [
          {
            id: "seat",
            room_id: "room",
            row: 0,
            column: 0,
            number: 1,
            expected_ip: null,
            status: "unbound",
            devices: [],
          },
        ],
      },
    ],
  },
});
const onMutate = async () => {};
test("同一稳定座位按所选比赛展示选手，未分配有明确反馈", () => {
  assert.match(participantLabel(contest, "seat"), /001.*已分配/);
  assert.equal(
    participantLabel({ ...contest, id: "second", participants: [] }, "seat"),
    "未分配选手",
  );
  const html = renderToStaticMarkup(
    createElement(Seats, {
      snapshot,
      contest,
      csrf: "test",
      disabled: false,
      onMutate,
      selected: [],
      onSelect: () => {},
    }),
  );
  for (const text of ["合成甲", "合成学校", "001", "已分配"])
    assert.ok(html.includes(text));
  const list = renderToStaticMarkup(
    createElement(Seats, {
      snapshot,
      contest,
      csrf: "test",
      disabled: false,
      onMutate,
      selected: [],
      onSelect: () => {},
      targetMode: true,
    }),
  );
  assert.ok(list.includes("合成甲 · 合成学校 · 001 · 已分配"));
});
test("名单入口展示导入语义、确定性分配规则、操作历史和真实名单", () => {
  const html = renderToStaticMarkup(
    createElement(Roster, {
      snapshot,
      contest,
      csrf: "test",
      disabled: false,
      onMutate,
    }),
  );
  for (const text of [
    "UTF-8 TSV",
    "缺失行保留",
    "容量不足整批拒绝",
    "操作历史",
    "编辑与分配",
    "合成甲",
  ])
    assert.ok(html.includes(text));
  assert.equal(sampleRoster.split("\n")[0], "code\tname\tschool");
});
test("无比赛和断线状态提供空列表与禁止提交", () => {
  const html = renderToStaticMarkup(
    createElement(Roster, { snapshot, csrf: "test", disabled: true, onMutate }),
  );
  assert.match(html, /请创建或在页面顶部选择比赛/);
  assert.match(html, /disabled=""/);
});
test("预览契约保留错误行和修改前后，拒绝未知差异及不完整选手", () => {
  const preview = {
    contest_id: "first",
    name: "合成",
    base_version: 1,
    preview_token: "token",
    total: 1,
    rooms: [],
    changes: [
      {
        line: 2,
        kind: "modified",
        before: contest.participants[0],
        after: { ...contest.participants[0], name: "修正" },
      },
    ],
    errors: [{ line: 3, message: "重复标识" }],
    skipped: [],
  };
  assert.equal(rosterPreviewSchema.parse(preview).errors[0].line, 3);
  assert.equal(
    rosterPreviewSchema.safeParse({
      ...preview,
      changes: [{ ...preview.changes[0], kind: "unknown" }],
    }).success,
    false,
  );
  assert.equal(
    contestSchema.safeParse({ ...contest, participants: [{ code: "1" }] })
      .success,
    false,
  );
});
