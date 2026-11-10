import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { allowedActions, Enrollment, LifecycleControls } from "./Lifecycle";
import { deviceSchema, stateSchema } from "./contracts";
import { targetPreviewSchema } from "./seating";
import { DeviceDetails } from "./Devices";
import { ConfigurationEditor } from "./ConfigurationEditor";

const device = deviceSchema.parse({
  id: "synthetic",
  name: "合成设备",
  lifecycle: "enabled",
  lifecycle_version: 3,
  application_id: null,
  application_at: null,
  enrollment_expires: 0,
  enrollment_generation: 1,
  rotation_id: null,
  rotation_expires: null,
  registered: true,
  online: true,
  last_seen: 0,
  desired_job: "job",
  desired_version: 1,
  expired: false,
  stale: false,
  result: null,
  received_at: null,
});

test("生命周期操作符合合法转换，撤销后提供重新授权", () => {
  assert.deepEqual(allowedActions(device), ["disable", "rotate", "revoke"]);
  assert.deepEqual(allowedActions({ ...device, lifecycle: "disabled" }), [
    "enable",
    "rotate",
    "revoke",
  ]);
  assert.deepEqual(allowedActions({ ...device, lifecycle: "revoked" }), [
    "reauthorize",
  ]);
  assert.deepEqual(allowedActions({ ...device, lifecycle: "pending" }), [
    "reject",
  ]);
  assert.deepEqual(
    allowedActions({
      ...device,
      lifecycle: "pending",
      application_id: "claim",
    }),
    ["approve", "reject"],
  );
});
test("详情展示生命周期版本及身份代次，断线禁止写入", () => {
  const html = renderToStaticMarkup(
    createElement(LifecycleControls, {
      device,
      disabled: true,
      onMutate: async () => {},
    }),
  );
  assert.match(html, /已启用/);
  assert.match(html, /状态版本/);
  assert.match(html, /身份代次/);
  assert.match(html, /disabled=""/);
  assert.doesNotMatch(html, /enrollment_token/);
});
test("注册审核入口读取真实窗口状态并保留空列表筛选", () => {
  const snapshot = stateSchema.parse({
    protocol_version: 1,
    server_time: 0,
    version: 0,
    configuration: { schema_version: 1, config_version: "0", tools: [] },
    targets: [],
    catalog: [],
    devices: [],
    registration_window: { version: 1, closes_at: 0, open: false },
  });
  const html = renderToStaticMarkup(
    createElement(Enrollment, {
      snapshot,
      disabled: false,
      onMutate: async () => {},
    }),
  );
  assert.match(html, /已关闭/);
  assert.match(html, /开启注册窗口 1 小时/);
  assert.match(html, /申请筛选/);
});
test("座位目标预览识别设备禁用，未知生命周期拒绝", () => {
  assert.equal(
    targetPreviewSchema.parse({
      preview_token: "test",
      targets: [
        {
          seat_id: "seat",
          number: 1,
          device_id: "synthetic",
          device_ids: ["synthetic"],
          status: "disabled",
        },
      ],
    }).targets[0].status,
    "disabled",
  );
  assert.equal(
    deviceSchema.safeParse({ ...device, lifecycle: "unknown" }).success,
    false,
  );
});

test("重新授权设备按配置范围提供重查，撤销目标可取消选择", () => {
  const html = renderToStaticMarkup(
    createElement(DeviceDetails, {
      device: { ...device, desired_job: null },
      disabled: false,
      recheckAllowed: true,
      onRecheck: async () => {},
    }),
  );
  assert.match(html, /<button>重新检查<\/button>/);
  const snapshot = stateSchema.parse({
    protocol_version: 1,
    server_time: 0,
    version: 1,
    configuration: { schema_version: 1, config_version: "1", tools: [] },
    targets: [device.id],
    catalog: [],
    devices: [{ ...device, lifecycle: "revoked", registered: false }],
    registration_window: { version: 1, closes_at: 0, open: false },
  });
  const editor = renderToStaticMarkup(
    createElement(ConfigurationEditor, {
      snapshot,
      disabled: false,
      onReload: () => {},
      onPublish: async () => {},
    }),
  );
  const checkbox = editor.match(/<input[^>]*type="checkbox"[^>]*>/)?.[0];
  assert.ok(checkbox);
  assert.doesNotMatch(checkbox, /disabled/);
  assert.match(checkbox, /checked/);
});
