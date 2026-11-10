export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message);
  }
}
export async function api(
  path: string,
  body?: unknown,
  csrf?: string,
): Promise<unknown> {
  const response = await fetch(`/api/${path}`, {
    method: body === undefined ? "GET" : "POST",
    credentials: "same-origin",
    headers:
      body === undefined
        ? {}
        : { "Content-Type": "application/json", "X-CSRF-Token": csrf ?? "" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(12000),
  }).catch(() => {
    throw new ApiError("无法连接服务，请检查网络或等待服务恢复", 0);
  });
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
    throw new ApiError(message, response.status);
  }
  return response.json();
}
