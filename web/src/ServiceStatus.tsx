import { useState } from "react";

async function probe(path: "health" | "ready") {
  try {
    const response = await fetch(`/api/${path}`, {
      credentials: "omit",
      cache: "no-store",
      redirect: "error",
      signal: AbortSignal.timeout(6000),
    });
    const result = await response.json();
    if (result?.service === "wccs") {
      if (path === "health" && response.ok && result.protocol_version === 1)
        return "服务进程已响应";
      if (path === "ready" && response.ok && result.ready === true)
        return "管理数据库已就绪";
      if (path === "ready" && response.status === 503 && result.ready === false)
        return "管理数据库未就绪";
    }
  } catch {
    // An unreachable origin or a proxy error cannot confirm either observation.
  }
  return path === "health" ? "未确认服务进程" : "未确认数据库状态";
}

export function ServiceStatus() {
  const [result, setResult] = useState("");
  const [checking, setChecking] = useState(false);
  async function check() {
    setChecking(true);
    setResult("");
    try {
      const observations = await Promise.all([probe("health"), probe("ready")]);
      setResult(observations.join(" · "));
    } finally {
      setChecking(false);
    }
  }
  return (
    <details>
      <summary>服务入口诊断</summary>
      <p>{location.origin}</p>
      <button type="button" disabled={checking} onClick={() => void check()}>
        {checking ? "正在检查…" : "检查当前入口"}
      </button>
      <p role="status">{result}</p>
    </details>
  );
}
