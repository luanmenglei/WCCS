import { Waiting } from "./Waiting";
import { AccessContext, roleLabels } from "./access";
import { Operators } from "./Operators";
import { Events } from "./Events";
import { Roster } from "./Roster";
import { ZodError } from "zod";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { api, ApiError, advanceSession } from "./api";
import {
  sessionSchema,
  stateSchema,
  type Session,
  type Snapshot,
} from "./contracts";
import { ConfigurationEditor } from "./ConfigurationEditor";
import { Rooms } from "./Rooms";
import { Seats } from "./Seats";
import { pages, pageFromHash } from "./fleet";
import { Enrollment } from "./Lifecycle";
import { Devices } from "./Devices";

function message(error: unknown): string {
  if (error instanceof ZodError)
    return "服务响应格式不兼容，请检查服务与管理页面版本";
  return error instanceof Error ? error.message : "操作失败，请重试";
}

export function App() {
  const [contestId, setContestId] = useState("");
  const [page, setPage] = useState(() => pageFromHash(location.hash));
  const [seatSelection, setSeatSelection] = useState<string[]>([]);
  const [menuOpen, setMenuOpen] = useState(false);
  const sessionEpoch = useRef(0);
  const refreshSequence = useRef(0);
  const pageInfo = pages[page];
  useEffect(() => {
    function navigate() {
      setPage(pageFromHash(location.hash));
      const linkedContest = new URLSearchParams(
        location.hash.split("?")[1],
      ).get("contest_id");
      if (linkedContest) setContestId(linkedContest);
      setMenuOpen(false);
      window.scrollTo({ top: 0 });
    }
    window.addEventListener("hashchange", navigate);
    return () => window.removeEventListener("hashchange", navigate);
  }, []);
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
    function expire(event: Event) {
      sessionEpoch.current++;
      advanceSession();
      requestIds.current.clear();
      setSession(null);
      setSnapshot(null);
      setSeatSelection([]);
      setContestId("");
      setMenuOpen(false);
      setNotice("");
      setBusy(false);
      setConnected(false);
      setConnectionError("");
      setError((event as CustomEvent<string>).detail);
    }
    window.addEventListener("wccs-session-expired", expire);
    return () => window.removeEventListener("wccs-session-expired", expire);
  }, []);

  useEffect(() => {
    let active = true;
    api("session")
      .then((value) => {
        if (active) {
          const authenticated = sessionSchema.parse(value);
          advanceSession(authenticated.csrf);
          setSession(authenticated);
        }
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
    const epoch = sessionEpoch.current;
    const sequence = ++refreshSequence.current;
    try {
      const value = stateSchema.parse(await api("state"));
      if (epoch !== sessionEpoch.current) return value;
      if (sequence === refreshSequence.current) {
        setSnapshot(value);
        setConnected(true);
        setConnectionError("");
      }
      return value;
    } catch (error) {
      if (
        epoch !== sessionEpoch.current ||
        sequence !== refreshSequence.current
      )
        return;
      setConnected(false);
      if (error instanceof ApiError && error.status === 401) {
        sessionEpoch.current++;
        advanceSession(); // Retire in-flight reads from the previous login.
        requestIds.current.clear();
        setSeatSelection([]);
        setContestId("");
        setMenuOpen(false);
        setSession(null);
        setSnapshot(null);
        setNotice("");
        setBusy(false);
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
    const epoch = sessionEpoch.current;
    setBusy(true);
    setError("");
    setNotice("");
    const key = JSON.stringify([path, body]);
    const requestId =
      typeof body.request_id === "string"
        ? body.request_id
        : (requestIds.current.get(key) ?? crypto.randomUUID());
    requestIds.current.set(key, requestId);
    try {
      await api(path, { ...body, request_id: requestId }, session.csrf);
      if (epoch !== sessionEpoch.current) return;
      await refresh();
      if (epoch !== sessionEpoch.current) return;
      requestIds.current.delete(key);
      setNotice(success);
    } catch (error) {
      if (epoch === sessionEpoch.current) {
        setError(message(error));
        if (error instanceof ApiError && error.status === 401) {
          sessionEpoch.current++;
          advanceSession();
          requestIds.current.clear();
          setMenuOpen(false);
          setNotice("");
          setConnected(false);
          setSession(null);
          setSnapshot(null);
          setSeatSelection([]);
          setContestId("");
          setBusy(false);
        }
      }
      throw error;
    } finally {
      if (epoch === sessionEpoch.current) setBusy(false);
    }
  }

  async function login(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError("");
    const form = new FormData(event.currentTarget);
    try {
      const authenticated = sessionSchema.parse(
        await api("login", {
          username: form.get("username"),
          password: form.get("password"),
        }),
      );
      sessionEpoch.current++;
      advanceSession(authenticated.csrf);
      requestIds.current.clear();
      setSeatSelection([]);
      setContestId("");
      setMenuOpen(false);
      setNotice("");
      setConnectionError("");
      setSession(authenticated);
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
          <h1>赛场管理</h1>
          <p>Windows Contest Control System</p>
        </section>
        <form className="login-form" onSubmit={(event) => void login(event)}>
          <h2>操作员登录</h2>
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
    <AccessContext.Provider value={session}>
      <div className="shell" key={session.csrf}>
        <aside className={`sidebar ${menuOpen ? "is-open" : ""}`}>
          <div className="brand">
            <span className="brand-mark">W</span>
            <span>
              WCCS<small>赛场管理</small>
            </span>
          </div>
          <nav aria-label="页面导航" id="main-navigation">
            {["赛场管理", "赛前准备", "系统管理"].map((group) => (
              <div className="nav-group" key={group}>
                <p>{group}</p>
                {Object.entries(pages)
                  .filter(
                    ([id, info]) =>
                      info.group === group &&
                      (id !== "operators" || session.role === "admin"),
                  )
                  .map(([id, info]) => (
                    <a
                      key={id}
                      href={`#/${id}`}
                      aria-current={page === id ? "page" : undefined}
                      onClick={() => setMenuOpen(false)}
                    >
                      {info.label}
                    </a>
                  ))}
              </div>
            ))}
          </nav>
          <div className="account">
            <span>
              {session.display_name} · {session.username}
            </span>
            <small>
              {roleLabels[session.role]} · 权限 v{session.version}
            </small>
            <small>
              {session.all_rooms
                ? "全场授权"
                : `授权机房 ${session.room_ids.map((id) => snapshot?.seating.rooms.find((r) => r.id === id)?.name ?? id).join("、") || "无"}`}
            </small>
            <button
              disabled={busy}
              onClick={() => {
                void api("logout", {}, session.csrf)
                  .then(() => {
                    sessionEpoch.current++;
                    advanceSession();
                    requestIds.current.clear();
                    setSeatSelection([]);
                    setContestId("");
                    setMenuOpen(false);
                    setNotice("");
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
          <div className="topbar">
            <button
              className="mobile-nav"
              aria-expanded={menuOpen}
              aria-controls="main-navigation"
              onClick={() => setMenuOpen(!menuOpen)}
            >
              导航
            </button>
            <span>{pageInfo.group}</span>
            <span aria-hidden="true">›</span>
            <strong>{pageInfo.label}</strong>
          </div>
          <header className="page-header">
            <div>
              <h1>{pageInfo.label}</h1>
              <p className="page-description">{pageInfo.description}</p>
            </div>
            <div className="connection">
              <span className={connected ? "dot online" : "dot"} />
              <span>
                {connected
                  ? "服务已连接 · 每 3 秒刷新"
                  : "连接中断 · 状态已过期"}
                <small>
                  {snapshot
                    ? `更新于 ${new Date(snapshot.server_time * 1000).toLocaleTimeString("zh-CN", { hour12: false })}`
                    : "等待状态快照"}
                </small>
              </span>
              <button
                disabled={busy}
                onClick={() =>
                  void refresh().catch((error) =>
                    setConnectionError(message(error)),
                  )
                }
              >
                立即刷新
              </button>
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
          {snapshot && (
            <label className="panel contest-context">
              当前比赛
              <select
                aria-label="当前比赛"
                value={contestId}
                disabled={busy}
                onChange={(event) => setContestId(event.target.value)}
              >
                <option value="">请选择比赛</option>
                {snapshot.roster.contests.map((contest) => (
                  <option key={contest.id} value={contest.id}>
                    {contest.name} · {contest.id}
                  </option>
                ))}
              </select>
            </label>
          )}
          {snapshot ? (
            <>
              {session.role !== "admin" && (
                <p className="banner">
                  {session.role === "viewer"
                    ? "当前身份只读。"
                    : "可在授权范围启停设备、重查工具及控制等待界面。"}{" "}
                  注册材料、机房/IP、全局名单和工具发布需要管理员权限。完整名单历史需要全场授权。
                </p>
              )}
              {page === "operators" &&
                (session.role === "admin" ? (
                  <Operators
                    snapshot={snapshot}
                    session={session}
                    onMutate={mutate}
                    disabled={busy || !connected}
                  />
                ) : (
                  <p role="alert">操作员管理需要管理员权限。</p>
                ))}
              {page === "events" && <Events />}
              {page === "waiting" && (
                <Waiting
                  key={contestId}
                  snapshot={snapshot}
                  contestId={contestId}
                  selected={seatSelection}
                  onSelect={setSeatSelection}
                  csrf={session.csrf}
                  disabled={busy || !connected || session.role === "viewer"}
                  onMutate={mutate}
                />
              )}
              <div className="page-view" hidden={page !== "roster"}>
                <Roster
                  key={contestId}
                  snapshot={snapshot}
                  contest={snapshot.roster.contests.find(
                    (c) => c.id === contestId,
                  )}
                  csrf={session.csrf}
                  disabled={busy || !connected || session.role !== "admin"}
                  onMutate={mutate}
                />
              </div>
              <div className="page-view" hidden={page !== "seats"}>
                <Seats
                  contest={snapshot.roster.contests.find(
                    (c) => c.id === contestId,
                  )}
                  selected={seatSelection}
                  onSelect={setSeatSelection}
                  snapshot={snapshot}
                  csrf={session.csrf}
                  disabled={busy || !connected || session.role === "viewer"}
                  onMutate={mutate}
                />
              </div>
              <div className="page-view" hidden={page !== "targets"}>
                <Seats
                  contest={snapshot.roster.contests.find(
                    (c) => c.id === contestId,
                  )}
                  selected={seatSelection}
                  onSelect={setSeatSelection}
                  snapshot={snapshot}
                  csrf={session.csrf}
                  disabled={busy || !connected || session.role === "viewer"}
                  onMutate={mutate}
                  targetMode
                />
              </div>
              <div className="page-view" hidden={page !== "rooms"}>
                <Rooms
                  snapshot={snapshot}
                  csrf={session.csrf}
                  disabled={busy || !connected || session.role !== "admin"}
                  onMutate={mutate}
                />
              </div>
              <div className="page-view" hidden={page !== "configuration"}>
                <ConfigurationEditor
                  key={editorKey}
                  snapshot={snapshot}
                  disabled={busy || !connected || session.role !== "admin"}
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
              </div>
              <div className="page-view" hidden={page !== "enrollment"}>
                <Enrollment
                  snapshot={snapshot}
                  disabled={busy || !connected || session.role !== "admin"}
                  onMutate={mutate}
                />
              </div>
              <div className="page-view" hidden={page !== "devices"}>
                <Devices
                  contest={snapshot.roster.contests.find(
                    (c) => c.id === contestId,
                  )}
                  onMutate={mutate}
                  snapshot={snapshot}
                  disabled={busy || !connected || session.role === "viewer"}
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
              </div>
            </>
          ) : (
            <p role="status">正在读取设备与配置…</p>
          )}
        </main>
      </div>
    </AccessContext.Provider>
  );
}
