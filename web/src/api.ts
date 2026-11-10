export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message);
  }
}
let generation = 0;
let controller = new AbortController();
let sessionBinding = "";
export function advanceSession(csrf = "") {
  generation++;
  sessionBinding = csrf;
  controller.abort();
  controller = new AbortController();
}
export async function api(
  path: string,
  body?: unknown,
  csrf?: string,
): Promise<unknown> {
  const owner = generation;
  const response = await fetch(`/api/${path}`, {
    method: body === undefined ? "GET" : "POST",
    credentials: "same-origin",
    headers: {
      ...(path !== "login" && sessionBinding
        ? { "X-WCCS-Session": sessionBinding }
        : {}),
      ...(body === undefined
        ? {}
        : { "Content-Type": "application/json", "X-CSRF-Token": csrf ?? "" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.any([AbortSignal.timeout(12000), controller.signal]),
  }).catch(() => {
    throw new ApiError("无法连接服务，请检查网络或等待服务恢复", 0);
  });
  if (owner !== generation) throw new ApiError("旧会话响应已失效", 0);
  if (!response.ok) {
    const text = await response.text();
    let message = `请求失败（${response.status}）`;
    try {
      const value: unknown = JSON.parse(text);
      if (
        typeof value === "object" &&
        value &&
        "error" in value &&
        typeof value.error === "string"
      )
        message = value.error;
    } catch {
      /* Non-JSON proxy errors use the HTTP status. */
    }
    if (owner !== generation) throw new ApiError("旧会话响应已失效", 0);
    if (response.status === 401 && path !== "login")
      window.dispatchEvent(
        new CustomEvent("wccs-session-expired", { detail: message }),
      );
    throw new ApiError(message, response.status);
  }
  const value: unknown = await response.json();
  if (owner !== generation) throw new ApiError("旧会话响应已失效", 0);
  return value;
}
