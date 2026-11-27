import { useAccess } from "./access";
import { Preparation } from "./Preparation";
import { useEffect, useState } from "react";
import type { z } from "zod";
import { api, type Mutation } from "./api";
import type { Snapshot } from "./contracts";
import {
  maintenancePreviewSchema,
  maintenanceListSchema,
  maintenanceReasons,
  maintenanceStages,
  maintenanceDetails,
  maintenanceTemplates,
} from "./maintenance-model";

export function Maintenance({
  snapshot,
  csrf,
  disabled,
  onMutate,
}: {
  snapshot: Snapshot;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const access = useAccess();
  const [selected, setSelected] = useState<string[]>(() => {
    const id = new URLSearchParams(location.hash.split("?")[1]).get(
      "device_id",
    );
    return id ? [id] : [];
  });
  const [profileVersion, setProfileVersion] = useState(0);
  const [operation, setOperation] = useState(() =>
    access.manage &&
    new URLSearchParams(location.hash.split("?")[1]).get("operation") ===
      "shutdown"
      ? "shutdown"
      : "identify",
  );
  const [duration, setDuration] = useState(30);
  const [preview, setPreview] = useState<z.infer<
    typeof maintenancePreviewSchema
  > | null>(null);
  const [request, setRequest] = useState<Record<string, unknown> | null>(null);
  const [jobs, setJobs] = useState<
    z.infer<typeof maintenanceListSchema>["jobs"]
  >([]);
  const [fresh, setFresh] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [reviewed, setReviewed] = useState("");
  const selection = JSON.stringify([
    selected,
    duration,
    operation,
    profileVersion,
  ]);
  const current = reviewed === selection ? preview : null;
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const response = maintenanceListSchema.parse(await api("maintenance"));
        if (active) {
          setJobs(response.jobs);
          setFresh(true);
        }
      } catch {
        if (active) setFresh(false);
      }
      if (active) timer = setTimeout(() => void poll(), 2000);
    }
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, []);
  async function review() {
    setBusy(true);
    setError("");
    setPreview(null);
    const body = {
      request_id: crypto.randomUUID(),
      template_version: 1,
      targets: selected,
      template:
        operation === "identify"
          ? { operation, duration_seconds: duration }
          : operation === "shutdown"
            ? { operation }
            : { operation, profile_version: profileVersion },
    };
    try {
      const value = maintenancePreviewSchema.parse(
        await api("maintenance/preview", body, csrf),
      );
      setPreview(value);
      setRequest({ ...body, preview_token: value.preview_token });
      setReviewed(selection);
    } catch (error) {
      setError(error instanceof Error ? error.message : "预览失败");
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <Preparation
        toolVersion={snapshot.version}
        disabled={disabled}
        onMutate={onMutate}
        onVersion={setProfileVersion}
      />
      <section className="panel operations-panel">
        <h2>固定维护模板</h2>
        <label>
          操作模板
          <select
            value={operation}
            onChange={(e) => setOperation(e.target.value)}
          >
            {access.manage && <option value="shutdown">统一关机</option>}
            <option value="identify">设备识别</option>
            <option value="prepare_desktop">创建或更新桌面入口</option>
            <option value="check_workspace">检查普通账户工作目录</option>
            <option value="deploy_scripts">部署并核验脚本包</option>
          </select>
        </label>
        {operation === "identify" && (
          <>
            <p>
              在选定设备显示座位、IP 和设备编号。工作人员可按 Esc
              关闭，到期自动关闭。
            </p>
            <label>
              显示秒数
              <input
                type="number"
                min={5}
                max={120}
                value={duration}
                onChange={(e) => setDuration(Number(e.target.value))}
              />
            </label>
          </>
        )}
        <fieldset className="maintenance-targets" disabled={disabled || busy}>
          <legend>目标设备</legend>
          {operation === "shutdown" && (
            <>
              <p>
                Windows 接受请求后将于 30
                秒后开始关机。请先让选手保存文件，并在收到逐机回执后关闭服务器。
              </p>
              <button
                onClick={() =>
                  setSelected(
                    snapshot.devices.filter((d) => d.online).map((d) => d.id),
                  )
                }
              >
                选择全部在线设备
              </button>
              <button onClick={() => setSelected([])}>清空选择</button>
            </>
          )}
          {snapshot.devices.map((device) => (
            <label key={device.id}>
              <input
                type="checkbox"
                checked={selected.includes(device.id)}
                onChange={(e) =>
                  setSelected(
                    e.target.checked
                      ? [...selected, device.id]
                      : selected.filter((id) => id !== device.id),
                  )
                }
              />
              {device.name} · {device.id} · {device.online ? "在线" : "离线"}
            </label>
          ))}
        </fieldset>
        <button
          disabled={disabled || busy || !selected.length}
          onClick={() => void review()}
        >
          预览目标
        </button>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        {current && (
          <>
            {current.preparation && (
              <div>
                <p>
                  维护 v{current.preparation.version} · 工具配置 v
                  {current.preparation.profile.tool_version} · 执行账户
                  SYSTEM，访问检查使用已核验普通比赛账户。
                </p>
                <p>
                  工作目录 {current.preparation.profile.work_directory} ·
                  桌面目录 {current.preparation.profile.desktop_directory} ·
                  脚本目录 {current.preparation.profile.script_directory}
                </p>
                {current.preparation.tools.map((t) => (
                  <p key={t.id}>
                    {t.id} · {t.path}
                  </p>
                ))}
              </div>
            )}
            <table>
              <thead>
                <tr>
                  <th>设备</th>
                  <th>座位 / IP</th>
                  <th>执行条件</th>
                </tr>
              </thead>
              <tbody>
                {current.targets.map((target) => (
                  <tr key={target.device_id}>
                    <td>
                      {target.name} · {target.device_id}
                    </td>
                    <td>
                      {target.identity.seat_label} · {target.identity.school}
                    </td>
                    <td>
                      {target.reason
                        ? (maintenanceReasons[target.reason] ?? target.reason)
                        : "可执行"}{" "}
                      ·{" "}
                      {target.adapter === "local_test"
                        ? "开发机测试"
                        : "SYSTEM 服务"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <button
              className="primary"
              disabled={
                disabled ||
                busy ||
                !request ||
                !current.targets.some((t) => !t.reason)
              }
              onClick={() => {
                if (!request) return;
                setBusy(true);
                void onMutate(
                  "maintenance/commit",
                  request,
                  "已下发固定维护模板，请核对逐机实际回执。",
                )
                  .then(() => {
                    setPreview(null);
                    setRequest(null);
                  })
                  .catch((error) =>
                    setError(
                      error instanceof Error ? error.message : "提交失败",
                    ),
                  )
                  .finally(() => setBusy(false));
              }}
            >
              {operation === "shutdown"
                ? `确认关闭 ${current.targets.filter((t) => !t.reason).length} 台电脑`
                : "执行所选模板"}
            </button>
          </>
        )}
      </section>
      <section className="panel operations-panel">
        <h2>逐机维护结果</h2>
        {!fresh && <p role="status">结果连接中断，显示的数据已过期。</p>}
        <p>最近 500 项任务</p>
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>设备</th>
                <th>阶段</th>
                <th>实际结果</th>
                <th>关联记录</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {jobs.map((job) => (
                <tr key={job.job_id}>
                  <td>
                    <a
                      href={`#/devices?device_id=${encodeURIComponent(job.device_id)}`}
                    >
                      {job.device_id}
                    </a>
                    <p>
                      {maintenanceTemplates[job.command.template.operation] ??
                        job.command.template.operation}
                    </p>
                    <small>
                      {new Date(job.updated_at * 1000).toLocaleString("zh-CN")}
                    </small>
                  </td>
                  <td>
                    {maintenanceStages[job.stage] ?? job.stage}
                    {job.expired &&
                    ["queued", "accepted", "executing"].includes(job.stage)
                      ? " · 授权已到期，等待终态补报"
                      : ""}
                    {job.cancel_requested ? " · 已请求取消" : ""}
                  </td>
                  <td>
                    {job.result
                      ? (maintenanceDetails[job.result.detail] ??
                        job.result.detail)
                      : "尚无客户端回执"}
                    {job.result?.steps.map((step, index) => (
                      <p key={index}>
                        {step.phase} · {step.outcome} · {step.path} ·{" "}
                        {step.detail}
                      </p>
                    ))}
                  </td>
                  <td>
                    <a
                      href={`#/events?request_id=${encodeURIComponent(job.request_id)}`}
                    >
                      {job.request_id}
                    </a>
                    <small className="mono">{job.job_id}</small>
                  </td>
                  <td>
                    <button
                      disabled={
                        disabled ||
                        busy ||
                        job.cancel_requested ||
                        !["queued", "accepted", "executing"].includes(job.stage)
                      }
                      onClick={() => {
                        void onMutate(
                          "maintenance/cancel",
                          { job_id: job.job_id },
                          "已请求取消，等待客户端停止回执。",
                        ).catch((error) =>
                          setError(
                            error instanceof Error ? error.message : "取消失败",
                          ),
                        );
                      }}
                    >
                      取消任务
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}
