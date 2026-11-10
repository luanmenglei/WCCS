import { test } from "node:test";
import assert from "node:assert/strict";
import {
  actualLabel,
  waitingItemSchema,
  waitingPreviewSchema,
} from "./waiting-model";
import { pageFromHash } from "./fleet";

const command = {
  protocol_version: 1,
  command_id: "command",
  request_id: "request",
  sequence: 1,
  device_id: "device",
  generation: 2,
  device_version: 3,
  instance_id: "instance",
  session_key: "session",
  action: "enter",
  expires_at: 1000,
  identity: {
    contest_id: "contest",
    contest_name: "合成比赛",
    contest_version: 1,
    participant_id: "participant",
    name: "林小禾",
    school: "合成学校",
    seat_id: "seat",
    seat_label: "A / 1",
    room_id: "A",
    room_version: 2,
  },
};
test("waiting route and receipt preserve exact identity and generation", () => {
  assert.equal(pageFromHash("#/waiting"), "waiting");
  const item = waitingItemSchema.parse({
    command,
    stage: "accepted",
    report: null,
    online: false,
    actual: null,
    last_seen: null,
    updated_at: 100,
  });
  assert.equal(item.command.identity.name, "林小禾");
  assert.equal(item.command.generation, 2);
  assert.equal(actualLabel(item), "离线 · 实际状态未知");
});
test("completed receipt never implies fresh actual waiting", () => {
  const item = waitingItemSchema.parse({
    command,
    stage: "succeeded",
    report: null,
    online: true,
    actual: {
      state: "failed",
      command_id: null,
      session_key: null,
      monitors: 0,
      detail: "session_exited",
    },
    last_seen: 100,
    updated_at: 100,
  });
  assert.equal(actualLabel(item), "执行或恢复失败");
  assert.equal(actualLabel({ ...item, online: false }), "离线 · 实际状态未知");
});
test("waiting previews reject unsupported actions and unknown skip states", () => {
  const preview = {
    preview_token: "digest",
    contest_id: "contest",
    contest_name: "合成",
    contest_version: 1,
    action: "enter",
    duration_seconds: 900,
    targets: [],
  };
  assert.equal(waitingPreviewSchema.parse(preview).action, "enter");
  assert.equal(
    waitingPreviewSchema.safeParse({ ...preview, action: "reboot" }).success,
    false,
  );
  assert.equal(
    waitingItemSchema.safeParse({
      command: { ...command, protocol_version: 2 },
    }).success,
    false,
  );
});
