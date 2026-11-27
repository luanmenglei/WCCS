import { useState, useRef, useCallback, useEffect } from "react";
import { ZodError } from "zod";
import { api, ApiError, advanceSession } from "./api";
import {
  sessionSchema,
  stateSchema,
  type Session,
  type Snapshot,
} from "./contracts";

function message(error: unknown): string {
  if (error instanceof ZodError)
    return "服务响应格式不兼容，请检查服务与管理页面版本";
  return error instanceof Error ? error.message : "操作失败，请重试";
}

export function useAdminSession(onSessionChange: (signedIn: boolean) => void) {
  const sessionEpoch = useRef(0);
  const refreshSequence = useRef(0);
  const [session, setSession] = useState<Session | null>(null);
  const [checking, setChecking] = useState(true);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState("");
  const [connectionError, setConnectionError] = useState("");
  const [notice, setNotice] = useState("");
  const [receipt, setReceipt] = useState("");
  const [busy, setBusy] = useState(false);
  const requestIds = useRef(new Map<string, string>());
  const endSession = useCallback(
    (reason: string) => {
      // 先退役在途请求，再清空快照、回执和选中的目标，防止上次登录的响应回填页面。
      sessionEpoch.current++;
      advanceSession();
      requestIds.current.clear();
      setSession(null);
      setSnapshot(null);
      setReceipt("");
      setNotice("");
      setBusy(false);
      setConnected(false);
      setConnectionError("");
      setError(reason);
      onSessionChange(false);
    },
    [onSessionChange],
  );

  useEffect(() => {
    // api() 在返回 401 前同步通知所有页面，轮询与写请求走同一条失效路径。
    const expire = (event: Event) =>
      endSession((event as CustomEvent<string>).detail);
    window.addEventListener("wccs-session-expired", expire);
    return () => window.removeEventListener("wccs-session-expired", expire);
  }, [endSession]);

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
    // 手动刷新可与轮询重叠。只允许同一登录会话的最新请求更新快照。
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
    if (!session || !connected)
      throw new Error("管理状态尚未就绪，请先刷新核对。");
    const epoch = sessionEpoch.current;
    setBusy(true);
    setError("");
    setNotice("");
    setReceipt("");
    const key = JSON.stringify([path, body]);
    const requestId =
      typeof body.request_id === "string"
        ? body.request_id
        : (requestIds.current.get(key) ?? crypto.randomUUID());
    requestIds.current.set(key, requestId);
    try {
      await api(path, { ...body, request_id: requestId }, session.csrf);
      if (epoch !== sessionEpoch.current) return;
      requestIds.current.delete(key);
      setReceipt(requestId);
      setNotice(success);
      try {
        await refresh();
      } catch (refreshError) {
        if (epoch === sessionEpoch.current) {
          setNotice(
            `操作已受理（${requestId}）。状态刷新失败，请刷新核对，不要重新提交。`,
          );
          setConnectionError(message(refreshError));
        }
      }
    } catch (error) {
      if (epoch === sessionEpoch.current) {
        if (
          error instanceof ApiError &&
          (error.status === 0 || error.status >= 500)
        ) {
          setReceipt(requestId);
          setError(`未能确认请求 ${requestId} 的结果，请先核对任务记录。`);
        } else setError(message(error));
      }
      throw error;
    } finally {
      if (epoch === sessionEpoch.current) setBusy(false);
    }
  }

  async function login(form: FormData) {
    setBusy(true);
    setError("");
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
      onSessionChange(true);
      setNotice("");
      setConnectionError("");
      setSession(authenticated);
    } catch (error) {
      setError(message(error));
    } finally {
      setBusy(false);
    }
  }

  async function logout() {
    if (!session) return;
    const epoch = sessionEpoch.current;
    try {
      await api("logout", {}, session.csrf);
      if (epoch === sessionEpoch.current) endSession("");
    } catch (error) {
      if (epoch === sessionEpoch.current) setError(message(error));
    }
  }

  function refreshNow() {
    void refresh().catch((error) => setConnectionError(message(error)));
  }

  async function reloadConfiguration() {
    try {
      await refresh();
      setError("");
      return true;
    } catch (error) {
      setError(message(error));
      return false;
    }
  }

  return {
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
    dismissError: () => setError(""),
  };
}
