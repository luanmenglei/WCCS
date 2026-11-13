import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { diagnosticSchema, diagnosticSteps } from "./diagnostics-model";
import { Diagnostics } from "./Diagnostics";
import { deviceSchema } from "./contracts";

test("diagnostics distinguish partial success, old evidence and recovery actions", () => {
  const report = diagnosticSchema.parse({
    schema_version: 1,
    request_id: "request",
    device_id: "device",
    agent_version: "0.3.0",
    evidence_context: "local_test",
    configuration_revision: 2,
    configuration_digest: "a".repeat(64),
    collected_at: 1,
    seat_claim: null,
    checks: Object.keys(diagnosticSteps).map((step) => ({
      step,
      outcome: step === "configuration" ? "succeeded" : "pending",
      checked_at: 1,
      last_success_at: step === "configuration" ? 1 : null,
      code:
        step === "configuration" ? "saved_offline" : "connection_unavailable",
    })),
  });
  const device = deviceSchema.parse({
    id: "device",
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
    last_seen: 1,
    desired_job: null,
    desired_version: null,
    expired: false,
    stale: false,
    result: null,
    received_at: null,
    diagnostics: report,
    diagnostics_received_at: 1,
  });
  const html = renderToStaticMarkup(createElement(Diagnostics, { device }));
  for (const text of [
    "开发机测试",
    "数据已过期",
    "待联网验证",
    "教学离线可保持待机",
    "HTTP 心跳",
  ])
    assert.ok(html.includes(text));
  assert.throws(() =>
    diagnosticSchema.parse({
      ...report,
      checks: [{ ...report.checks[0], code: "Bearer secret" }],
    }),
  );
});
