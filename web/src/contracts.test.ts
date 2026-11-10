import { readFileSync } from "node:fs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { checkLabel, stateSchema, resultSchema } from "./contracts.ts";

test("运行时拒绝未知协议和不完整状态", () => {
  assert.equal(stateSchema.safeParse({ protocol_version: 99 }).success, false);
  assert.equal(
    stateSchema.safeParse({ protocol_version: 1, devices: [] }).success,
    false,
  );
});
test("逐项失败保留稳定原因与系统码", () => {
  assert.match(
    checkLabel({ status: "unavailable", reason: "not_configured", code: null }),
    /not_configured/,
  );
  assert.match(
    checkLabel({ status: "unavailable", reason: "permission_denied", code: 5 }),
    /5/,
  );
});
test("Rust JSON 协议结果形状", () => {
  const result = resultSchema.parse(
    JSON.parse(
      readFileSync(
        new URL(
          "../../crates/wccs-protocol/tests/fixtures/result.json",
          import.meta.url,
        ),
        "utf8",
      ),
    ),
  );
  assert.equal(result.tools[0].tool.path, null);
  assert.equal(result.tools[1].tool.id, "shell");
});
