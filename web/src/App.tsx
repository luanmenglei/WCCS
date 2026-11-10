import { ZodError } from "zod";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { api, ApiError } from "./api";
import {
  sessionSchema,
  stateSchema,
  type Session,
  type Snapshot,
} from "./contracts";
import { ConfigurationEditor } from "./ConfigurationEditor";
import { Devices } from "./Devices";

function message(error: unknown): string {
  if (error instanceof ZodError)
    return "服务响应格式不兼容，请检查服务与管理页面版本";
  return error instanceof Error ? error.message : "操作失败，请重试";
}

export function App() {
  const [session, setSession] = useState<Session | null>(null);
  const [checking, setChecking] = useState(true);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState("");
  const [connectionError, setConnectionError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [editorKey, setEditorKey] = useState(0);
  const requestIds = useRef(new Map<string, string>());

  useEffect(() => {
    let active = true;
    api("session")
      .then((value) => {
        if (active) setSession(sessionSchema.parse(value));
      })
      .catch((error: unknown) => {
        if (active && !(error instanceof ApiError && error.status === 401))
          setError(message(error));
      })
      .finally(() => {
        if (active) setChecking(false);
      });
    return () => {
      active = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      const value = stateSchema.parse(await api("state"));
      setSnapshot(value);
      setConnected(true);
      setConnectionError("");
      return value;
    } catch (error) {
      setConnected(false);
      if (error instanceof ApiError && error.status === 401) {
        setSession(null);
        setSnapshot(null);
      }
      throw error;
    }
  }, []);

  useEffect(() => {
    if (!session) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        await refresh();
      } catch (error) {
        if (active) setConnectionError(message(error));
      }
      if (active)
        timer = setTimeout(() => {
          void poll();
        }, 3000);
    }
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [session, refresh]);

  async function mutate(
    path: string,
    body: Record<string, unknown>,
    success: string,
  ) {
    if (!session) return;
    setBusy(true);
    setError("");
    setNotice("");
    const key = JSON.stringify([path, body]);
    const requestId = requestIds.current.get(key) ?? crypto.randomUUID();
    requestIds.current.set(key, requestId);
    try {
      await api(
        path,
        path === "devices" ? body : { ...body, request_id: requestId },
        session.csrf,
      );
      requestIds.current.delete(key);
      await refresh();
      setNotice(success);
    } catch (error) {
      setError(message(error));
      throw error;
    } finally {
      setBusy(false);
    }
  }

  async function login(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError("");
    const form = new FormData(event.currentTarget);
    try {
      setSession(
        sessionSchema.parse(
          await api("login", {
            username: form.get("username"),
            password: form.get("password"),
          }),
        ),
      );
    } catch (error) {
      setError(message(error));
    } finally {
      setBusy(false);
    }
  }

  if (checking)
    return (
      <main className="login">
        <p role="status">正在检查管理员会话…</p>
      </main>
    );
  if (!session)
    return (
      <main className="login">
        <section className="login-intro">
          <div className="brand">WCCS</div>
          <h1>
            赛场工具，
            <br />
            统一配置。
          </h1>
          <p>Windows Contest Control System</p>
        </section>
        <form className="login-form" onSubmit={(event) => void login(event)}>
          <h2>管理员登录</h2>
          <label>
            用户名
            <input name="username" autoComplete="username" required />
          </label>
          <label>
            密码
            <input
              name="password"
              type="password"
              autoComplete="current-password"
              required
            />
          </label>
          {error && (
            <p role="alert" className="error">
              {error}
            </p>
          )}
          <button className="primary" disabled={busy}>
            {busy ? "登录中…" : "登录管理台"}
          </button>
        </form>
      </main>
    );

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">WCCS</div>
        <p className="sidebar-caption">赛场管理台</p>
        <nav aria-label="页面导航">
          <a href="#configuration">工具配置</a>
          <a href="#devices">设备与检查</a>
        </nav>
        <div className="account">
          <span>{session.username}</span>
          <button
            disabled={busy}
            onClick={() => {
              void api("logout", {}, session.csrf)
                .then(() => {
                  setSession(null);
                  setSnapshot(null);
                  setConnected(false);
                  setError("");
                })
                .catch((error) => setError(message(error)));
            }}
          >
            退出登录
          </button>
        </div>
      </aside>
      <main className="workspace">
        <header className="page-header">
          <div>
            <p className="eyebrow">环境准备</p>
            <h1>工具路径配置</h1>
          </div>
          <div className="connection">
            <span className={connected ? "dot online" : "dot"} />
            {connected ? "服务已连接" : "连接中断 · 状态已过期"}
          </div>
        </header>
        {(error || connectionError) && (
          <div role="alert" className="banner error">
            {error || connectionError}
            <button onClick={() => setError("")}>关闭提示</button>
          </div>
        )}
        {notice && (
          <p role="status" className="banner success">
            {notice}
          </p>
        )}
        {snapshot ? (
          <>
            <ConfigurationEditor
              key={editorKey}
              snapshot={snapshot}
              disabled={busy || !connected}
              onReload={() => {
                void refresh()
                  .then(() => {
                    setEditorKey((key) => key + 1);
                    setError("");
                  })
                  .catch((error) => setError(message(error)));
              }}
              onPublish={async (body) => {
                await mutate(
                  "configuration",
                  body,
                  "配置已发布，等待目标设备回传检查结果。",
                );
                setEditorKey((key) => key + 1);
              }}
            />
            <Devices
              snapshot={snapshot}
              disabled={busy || !connected}
              onCreate={async (body) => {
                await mutate(
                  "devices",
                  body,
                  "设备已创建，请将注册材料用于 Windows 检查入口。",
                );
              }}
              onRecheck={async (id) => {
                await mutate(
                  "recheck",
                  { base_version: snapshot.version, targets: [id] },
                  "重新检查任务已下发。",
                );
              }}
            />
          </>
        ) : (
          <p role="status">正在读取设备与配置…</p>
        )}
      </main>
    </div>
  );
}
