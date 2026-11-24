import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { AccessContext } from "./access";
import { LifecycleControls } from "./Lifecycle";
import { deviceSchema, sessionSchema, type Session } from "./contracts";
import { eventSchema, eventQuery } from "./Events";
import { operatorSchema } from "./Operators";
import { advanceSession, api } from "./api";
import { pageFromHash } from "./navigation";

const account: Session = {
  id: "operator",
  username: "operator",
  display_name: "合成操作员",
  csrf: "test",
  role: "operator",
  all_rooms: false,
  room_ids: ["A"],
  version: 2,
};
test("账号与事件契约保留稳定身份、权限版本和未知历史字段", () => {
  assert.equal(sessionSchema.parse(account).version, 2);
  assert.equal(
    sessionSchema.safeParse({ username: "old", csrf: "old" }).success,
    false,
  );
  assert.equal(
    operatorSchema.safeParse({ ...account, enabled: true, created_at: 0 })
      .success,
    true,
  );
  const event = eventSchema.parse({
    id: "1",
    source: "audit_events",
    source_id: "1",
    category: "historical",
    occurred_at: 0,
    actor_id: null,
    actor_name: "legacy",
    operation: "legacy",
    target: null,
    device_ids: [],
    room_ids: null,
    scope_kind: "unknown",
    contest_id: null,
    request_id: null,
    version: null,
    outcome: "unknown",
    detail: {},
    legacy: true,
  });
  assert.equal(event.actor_id, null);
  assert.equal(event.room_ids, null);
  assert.equal(pageFromHash("#/events?request_id=example"), "events");
  assert.equal(pageFromHash("#/operators"), "operators");
});
test("操作员仅显示启停操作，查看员没有生命周期写入按钮", () => {
  const device = deviceSchema.parse({
    id: "a",
    name: "A",
    lifecycle: "enabled",
    lifecycle_version: 1,
    application_id: null,
    application_at: null,
    enrollment_expires: 0,
    enrollment_generation: 1,
    rotation_id: null,
    rotation_expires: null,
    registered: true,
    online: true,
    last_seen: 0,
    desired_job: null,
    desired_version: null,
    expired: false,
    stale: false,
    result: null,
    received_at: null,
  });
  function render(role: Session["role"]) {
    return renderToStaticMarkup(
      createElement(
        AccessContext.Provider,
        { value: { ...account, role } },
        createElement(LifecycleControls, {
          device,
          disabled: false,
          onMutate: async () => {},
        }),
      ),
    );
  }
  assert.match(render("operator"), /禁用设备/);
  assert.doesNotMatch(render("operator"), /撤销凭据|更新凭据/);
  assert.doesNotMatch(render("viewer"), /<button/);
  assert.match(render("admin"), /撤销凭据/);
});
test("事件筛选使用精确字段并将本地时间转换为秒", () => {
  const query = new URLSearchParams(
    eventQuery({
      request_id: " same request ",
      room_id: "A",
      from: "2026-11-10T00:00:00Z",
      outcome: "failed",
    }),
  );
  assert.equal(query.get("request_id"), "same request");
  assert.equal(
    query.get("from"),
    String(Date.parse("2026-11-10T00:00:00Z") / 1000),
  );
  assert.throws(() => eventQuery({ from: "invalid" }));
});
test("旧会话迟到响应及响应体解析结果无法交给新身份", async () => {
  const original = globalThis.fetch;
  let finish!: (value: Response) => void;
  globalThis.fetch = () =>
    new Promise((resolve) => {
      finish = resolve;
    });
  try {
    const request = api("state");
    advanceSession();
    finish(
      new Response(JSON.stringify({ sensitive: "previous-session" }), {
        status: 200,
      }),
    );
    await assert.rejects(request, /旧会话响应已失效/);
  } finally {
    globalThis.fetch = original;
    advanceSession();
  }
});
