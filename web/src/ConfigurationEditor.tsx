import { useState, type FormEvent } from "react";
import type { Snapshot, Tool } from "./contracts";

type Props = {
  snapshot: Snapshot;
  disabled: boolean;
  onReload: () => void;
  onPublish: (body: Record<string, unknown>) => Promise<void>;
};

export function ConfigurationEditor({
  snapshot,
  disabled,
  onReload,
  onPublish,
}: Props) {
  const [baseVersion] = useState(snapshot.version);
  const [tools, setTools] = useState<Tool[]>(snapshot.configuration.tools);
  const [targets, setTargets] = useState(snapshot.targets);
  const [candidate, setCandidate] = useState("");
  const [custom, setCustom] = useState(false);
  const [localError, setLocalError] = useState("");
  const [dirty, setDirty] = useState(false);
  const changed = snapshot.version !== baseVersion;

  function update(id: string, change: Partial<Tool>) {
    setTools((items) =>
      items.map((item) => (item.id === id ? { ...item, ...change } : item)),
    );
    setDirty(true);
  }
  function addCustom(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const id = String(form.get("id")).trim();
    if (tools.some((tool) => tool.id === id)) {
      setLocalError("工具 ID 已存在");
      return;
    }
    setTools((items) => [
      ...items,
      {
        id,
        name: String(form.get("name")).trim(),
        kind: form.get("kind") as Tool["kind"],
        path: null,
      },
    ]);
    setCustom(false);
    setLocalError("");
    setDirty(true);
  }

  return (
    <section
      id="configuration"
      aria-labelledby="configuration-title"
      className="panel"
    >
      <div className="section-heading">
        <div>
          <h2 id="configuration-title">配置与下发</h2>
          <p>完整文件路径由管理员维护，留空项回传未配置。</p>
        </div>
        <div className="version">
          当前 v{snapshot.version}
          <span>
            编辑基于 v{baseVersion}
            {dirty ? " · 有未发布修改" : ""}
          </span>
        </div>
      </div>
      {changed && (
        <p className="banner warning" role="alert">
          其他操作已发布新版本。请重新加载最新配置后编辑。
        </p>
      )}
      <div className="configuration-layout">
        <div className="tool-editor">
          <div className="toolbar">
            <label className="candidate">
              候选工具
              <select
                aria-label="候选工具"
                value={candidate}
                onChange={(event) => setCandidate(event.target.value)}
                disabled={disabled}
              >
                <option value="">选择工具…</option>
                {snapshot.catalog
                  .filter(
                    (tool) =>
                      !tools.some((selected) => selected.id === tool.id),
                  )
                  .map((tool) => (
                    <option key={tool.id} value={tool.id}>
                      {tool.name}
                    </option>
                  ))}
              </select>
            </label>
            <button
              disabled={disabled || !candidate || tools.length >= 64}
              onClick={() => {
                const tool = snapshot.catalog.find(
                  (tool) => tool.id === candidate,
                );
                if (tool) setTools((items) => [...items, tool]);
                setCandidate("");
                setDirty(true);
              }}
            >
              添加候选
            </button>
            <button
              disabled={disabled || tools.length >= 64}
              onClick={() => setCustom(!custom)}
            >
              自定义工具
            </button>
          </div>
          {custom && (
            <form className="custom-form" onSubmit={addCustom}>
              <label>
                工具 ID
                <input
                  name="id"
                  pattern="[A-Za-z0-9_-]+"
                  maxLength={80}
                  required
                />
              </label>
              <label>
                显示名称
                <input name="name" maxLength={100} required />
              </label>
              <label>
                文件种类
                <select name="kind">
                  <option value="executable">可执行文件</option>
                  <option value="script">脚本</option>
                </select>
              </label>
              <button disabled={disabled}>添加工具</button>
            </form>
          )}
          {localError && (
            <p role="alert" className="error">
              {localError}
            </p>
          )}
          {tools.length === 0 && (
            <p className="empty">从候选清单添加工具，或创建自定义工具。</p>
          )}
          <div className="tool-list">
            {tools.map((tool) => (
              <div className="tool-row" key={tool.id}>
                <div className="tool-name">
                  <strong>{tool.name}</strong>
                  <small>
                    {tool.id} · {tool.kind === "script" ? "脚本" : "可执行文件"}
                  </small>
                </div>
                <label className="path-field">
                  <span>{tool.name}完整路径</span>
                  <input
                    aria-label={`${tool.name}完整路径`}
                    value={tool.path ?? ""}
                    disabled={disabled}
                    placeholder="未配置"
                    maxLength={1024}
                    onChange={(event) =>
                      update(tool.id, { path: event.target.value || null })
                    }
                    spellCheck={false}
                  />
                </label>
                <button
                  className="quiet"
                  aria-label={`移除${tool.name}`}
                  disabled={disabled}
                  onClick={() => {
                    setTools((items) =>
                      items.filter((item) => item.id !== tool.id),
                    );
                    setDirty(true);
                  }}
                >
                  移除
                </button>
              </div>
            ))}
          </div>
        </div>
        <fieldset className="targets" disabled={disabled}>
          <legend>目标设备 · {targets.length}</legend>
          {snapshot.devices.length === 0 ? (
            <p>先在下方新增设备并完成注册。</p>
          ) : (
            snapshot.devices.map((device) => (
              <label key={device.id} className="target">
                <input
                  type="checkbox"
                  disabled={!device.registered}
                  checked={targets.includes(device.id)}
                  onChange={(event) => {
                    setTargets((items) =>
                      event.target.checked
                        ? [...items, device.id]
                        : items.filter((id) => id !== device.id),
                    );
                    setDirty(true);
                  }}
                />
                <span>
                  <strong>{device.name}</strong>
                  <small>
                    {device.id} ·{" "}
                    {device.registered
                      ? device.online
                        ? "在线"
                        : "离线"
                      : "待注册"}
                  </small>
                </span>
              </label>
            ))
          )}
        </fieldset>
      </div>
      <footer className="panel-footer">
        <span>发布后生成新版本，向选中设备发送检查任务。</span>
        <div className="actions">
          <button disabled={disabled} onClick={onReload}>
            重新加载配置
          </button>
          <button
            className="primary"
            disabled={disabled || changed || targets.length === 0}
            onClick={() => {
              void onPublish({
                base_version: baseVersion,
                tools,
                targets,
              }).catch(() => {});
            }}
          >
            发布并检查
          </button>
        </div>
      </footer>
    </section>
  );
}
