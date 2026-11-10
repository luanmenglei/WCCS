import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DeviceDetails } from "./Devices";
import { ConfigurationEditor } from "./ConfigurationEditor";
import { stateSchema } from "./contracts";

const snapshot = stateSchema.parse({
  roster: { version: 0, contests: [] },
  registration_window: { version: 1, closes_at: 0, open: false },
  protocol_version: 1,
  server_time: 0,
  version: 1,
  configuration: { schema_version: 1, config_version: "1", tools: [] },
  targets: ["test"],
  catalog: [],
  devices: [
    {
      id: "test",
      name: "合成设备",
      lifecycle: "enabled",
      lifecycle_version: 1,
      application_id: null,
      application_at: null,
      enrollment_expires: 0,
      enrollment_generation: 1,
      rotation_id: null,
      rotation_expires: null,
      registered: true,
      online: false,
      last_seen: 0,
      desired_job: "new",
      desired_version: 1,
      expired: false,
      stale: true,
      result: {
        protocol_version: 1,
        device_id: "test",
        job_id: "old",
        config_version: "0",
        boot_id: "boot",
        tools: [
          {
            id: "submit",
            name: "submit",
            kind: "script",
            path: null,
            check: {
              status: "unavailable",
              reason: "not_configured",
              code: null,
            },
          },
        ],
      },
      received_at: 0,
    },
  ],
});

test("组件展示期望版本、实际版本和过期失败结果", () => {
  const html = renderToStaticMarkup(
    createElement(DeviceDetails, {
      device: snapshot.devices[0],
      disabled: false,
      onRecheck: async () => {},
    }),
  );
  for (const text of [
    "期望",
    "实际",
    "v1",
    "v0",
    "结果已过期",
    "not_configured",
    "离线",
  ])
    assert.ok(html.includes(text), text);
});
test("失联时配置组件禁用操作并保留当前目标", () => {
  const html = renderToStaticMarkup(
    createElement(ConfigurationEditor, {
      snapshot,
      disabled: true,
      onReload: () => {},
      onPublish: async () => {},
    }),
  );
  assert.match(html, /fieldset[^>]*disabled/);
  assert.match(html, /checked/);
  assert.match(html, /button[^>]*disabled[^>]*>发布并检查/);
});
