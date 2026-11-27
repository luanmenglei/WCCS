import { AccessContext, roleLabels } from "./access";
import { useCallback, useEffect, useState } from "react";
import { useAdminSession } from "./useAdminSession";
import { pages, pageFromHash, navigation } from "./navigation";
import { WorkspacePages } from "./WorkspacePages";
import { ServiceStatus } from "./ServiceStatus";

function linkedContestId() {
  return (
    new URLSearchParams(location.hash.split("?")[1]).get("contest_id") ?? ""
  );
}

export function App() {
  const [requestedContest, setContestId] = useState(linkedContestId);
  const [page, setPage] = useState(() => pageFromHash(location.hash));
  const [seatSelection, setSeatSelection] = useState<string[]>([]);
  const [menuOpen, setMenuOpen] = useState(false);
  const pageInfo = pages[page];
  useEffect(() => {
    function navigate() {
      setPage(pageFromHash(location.hash));
      const linkedContest = linkedContestId();
      if (linkedContest) setContestId(linkedContest);
      setMenuOpen(false);
      window.scrollTo({ top: 0 });
    }
    window.addEventListener("hashchange", navigate);
    return () => window.removeEventListener("hashchange", navigate);
  }, []);
  const resetWorkspace = useCallback((signedIn: boolean) => {
    setSeatSelection([]);
    setContestId(signedIn ? linkedContestId() : "");
    setMenuOpen(false);
  }, []);
  const {
    session,
    snapshot,
    checking,
    connected,
    busy,
    error,
    connectionError,
    notice,
    receipt,
    login,
    logout,
    mutate,
    refreshNow,
    reloadConfiguration,
    dismissError,
  } = useAdminSession(resetWorkspace);
  const contestId = snapshot?.roster.contests.some(
    (c) => c.id === requestedContest,
  )
    ? requestedContest
    : "";
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
          <ServiceStatus />
        </section>
        <form
          className="login-form"
          onSubmit={(event) => {
            event.preventDefault();
            void login(new FormData(event.currentTarget));
          }}
        >
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
            {navigation.map(({ group, routes }) => (
              <div className="nav-group" key={group}>
                <p>{group}</p>
                {routes
                  .filter(
                    (id) => id !== "operators" || session.role === "admin",
                  )
                  .map((id) => (
                    <a
                      key={id}
                      href={`#/${id}`}
                      aria-current={page === id ? "page" : undefined}
                      onClick={() => setMenuOpen(false)}
                    >
                      {pages[id].label}
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
            <button disabled={busy} onClick={() => void logout()}>
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
              <button disabled={busy} onClick={refreshNow}>
                立即刷新
              </button>
            </div>
          </header>
          <ServiceStatus />
          {!connected && snapshot && (
            <p role="status" className="banner warning">
              正在显示最后一次快照。逐机教学恢复情况需在重连后核对。
            </p>
          )}
          {(error || connectionError) && (
            <div role="alert" className="banner error">
              {error || connectionError}
              <button onClick={dismissError}>关闭提示</button>
            </div>
          )}
          {notice && (
            <p role="status" className="banner success">
              {notice}
            </p>
          )}
          {receipt && (notice || error) && (
            <a href={`#/events?request_id=${encodeURIComponent(receipt)}`}>
              按请求编号核对操作记录
            </a>
          )}
          {snapshot && (
            <label className="panel contest-context">
              当前比赛
              <select
                aria-label="当前比赛"
                value={contestId}
                disabled={busy}
                onChange={(event) => {
                  setSeatSelection([]);
                  setContestId(event.target.value);
                  const [route, search] = location.hash.split("?");
                  const params = new URLSearchParams(search);
                  if (event.target.value)
                    params.set("contest_id", event.target.value);
                  else params.delete("contest_id");
                  history.replaceState(
                    null,
                    "",
                    (route || "#/") +
                      (params.size ? "?" + params.toString() : ""),
                  );
                }}
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
            <WorkspacePages
              page={page}
              snapshot={snapshot}
              session={session}
              contestId={contestId}
              seatSelection={seatSelection}
              onSelect={setSeatSelection}
              disabled={busy || !connected}
              onMutate={mutate}
              onReloadConfiguration={reloadConfiguration}
            />
          ) : (
            <p role="status">正在读取设备与配置…</p>
          )}
        </main>
      </div>
    </AccessContext.Provider>
  );
}
