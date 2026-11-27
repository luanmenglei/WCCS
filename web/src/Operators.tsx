import { useEffect, useRef, useState } from "react";
import { z } from "zod";
import { api, type Mutation } from "./api";
import { roleLabels } from "./access";
import type { Session, Snapshot } from "./contracts";
import { DetailsDialog } from "./DetailsDialog";
import { randomSecret } from "./Lifecycle";

export const operatorSchema = z.object({
  id: z.string(),
  username: z.string(),
  display_name: z.string(),
  role: z.enum(["admin", "operator", "viewer"]),
  enabled: z.boolean(),
  all_rooms: z.boolean(),
  room_ids: z.array(z.string()),
  version: z.number(),
  created_at: z.number(),
});
type Operator = z.infer<typeof operatorSchema>;
function scope(account: Operator) {
  return account.all_rooms ? "全场" : account.room_ids.join("、") || "无机房";
}

export function Operators({
  snapshot,
  session,
  disabled,
  onMutate,
}: {
  snapshot: Snapshot;
  session: Session;
  disabled: boolean;
  onMutate: Mutation;
}) {
  const [accounts, setAccounts] = useState<Operator[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [search, setSearch] = useState("");
  const [editing, setEditing] = useState<Operator | null>(null);
  const [password, setPassword] = useState("");
  const [visible, setVisible] = useState(false);
  const [confirmation, setConfirmation] = useState<Record<
    string,
    unknown
  > | null>(null);
  const [receipt, setReceipt] = useState("");
  const sequence = useRef(0);
  async function reload() {
    const owner = ++sequence.current;
    setLoading(true);
    setError("");
    try {
      const value = z.array(operatorSchema).parse(await api("operators"));
      if (owner === sequence.current) setAccounts(value);
    } catch (error) {
      if (owner === sequence.current)
        setError(error instanceof Error ? error.message : "账号读取失败");
    } finally {
      if (owner === sequence.current) setLoading(false);
    }
  }
  useEffect(() => {
    const requestSequence = sequence;
    void reload();
    return () => {
      requestSequence.current++;
    };
  }, []);
  function open(account?: Operator) {
    setEditing(
      account ?? {
        id: crypto.randomUUID(),
        username: "",
        display_name: "",
        role: "viewer",
        enabled: true,
        all_rooms: false,
        room_ids: [],
        version: 0,
        created_at: 0,
      },
    );
    setPassword("");
    setVisible(false);
    setConfirmation(null);
    setError("");
  }
  function update(change: Partial<Operator>) {
    if (editing) setEditing({ ...editing, ...change });
    setConfirmation(null);
  }
  async function submit() {
    if (!confirmation) return;
    try {
      await onMutate(
        "operators",
        confirmation,
        "操作员变更已持久保存，相关旧会话已失效。",
      );
      setReceipt(String(confirmation.request_id));
      setPassword("");
      setEditing(null);
      setConfirmation(null);
      await reload();
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "提交失败，请使用同一确认重试",
      );
    }
  }
  const before = accounts.find((a) => a.id === editing?.id);
  return (
    <section className="panel">
      <div className="section-heading">
        <h2>操作员账号</h2>
        <div className="actions">
          <button disabled={disabled || loading} onClick={() => open()}>
            创建操作员
          </button>
          <button disabled={loading} onClick={() => void reload()}>
            重新加载账号
          </button>
        </div>
      </div>
      <p>
        管理员拥有全场权限。操作员可启停授权设备、重查工具。只读查看员可查询授权数据。机房/IP、名单、工具发布及凭据管理需要管理员权限。
      </p>
      <label>
        搜索账号或显示名
        <input value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      {loading && <p role="status">正在读取账号…</p>}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {receipt && (
        <p role="status">
          已保存，请求编号 <code>{receipt}</code> ·{" "}
          <a href={`#/events?request_id=${encodeURIComponent(receipt)}`}>
            查询变更事件
          </a>
        </p>
      )}
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th>账号</th>
              <th>显示名</th>
              <th>角色</th>
              <th>机房范围</th>
              <th>状态 / 版本</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {accounts
              .filter((a) =>
                `${a.username} ${a.display_name}`
                  .toLowerCase()
                  .includes(search.toLowerCase()),
              )
              .map((account) => (
                <tr key={account.id}>
                  <td>
                    {account.username}
                    {account.id === session.id && "（当前身份）"}
                  </td>
                  <td>{account.display_name}</td>
                  <td>{roleLabels[account.role]}</td>
                  <td>{scope(account)}</td>
                  <td>
                    {account.enabled ? "启用" : "禁用"} · v{account.version}
                  </td>
                  <td>
                    <button disabled={disabled} onClick={() => open(account)}>
                      配置权限
                    </button>
                  </td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>
      {!loading && !accounts.length && <p>暂无操作员账号。</p>}
      {editing && (
        <DetailsDialog
          title={editing.version ? `配置 ${editing.username}` : "创建操作员"}
          onClose={() => {
            setEditing(null);
            setPassword("");
            setConfirmation(null);
          }}
        >
          <form
            onSubmit={(e) => {
              e.preventDefault();
              setConfirmation({
                request_id: crypto.randomUUID(),
                id: editing.id,
                base_version: editing.version,
                username: editing.username,
                display_name: editing.display_name,
                role: editing.role,
                enabled: editing.enabled,
                all_rooms: editing.all_rooms,
                room_ids: editing.room_ids,
                ...(password ? { password } : {}),
              });
            }}
          >
            <label>
              账号名
              <input
                required
                minLength={editing.version ? undefined : 3}
                maxLength={64}
                disabled={!!editing.version || !!confirmation}
                value={editing.username}
                onChange={(e) => update({ username: e.target.value })}
              />
            </label>
            <p>账号名忽略大小写和首尾空白，创建后保持稳定。</p>
            <label>
              显示名
              <input
                required
                maxLength={100}
                disabled={!!confirmation}
                value={editing.display_name}
                onChange={(e) => update({ display_name: e.target.value })}
              />
            </label>
            <label>
              角色
              <select
                disabled={!!confirmation}
                value={editing.role}
                onChange={(e) => {
                  const role = e.target.value as Operator["role"];
                  update({
                    role,
                    ...(role === "admin"
                      ? { all_rooms: true, room_ids: [] }
                      : {}),
                  });
                }}
              >
                {Object.entries(roleLabels).map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <label>
              <input
                type="checkbox"
                checked={editing.enabled}
                disabled={!!confirmation}
                onChange={(e) => update({ enabled: e.target.checked })}
              />
              账号启用
            </label>
            <label>
              <input
                type="checkbox"
                checked={editing.all_rooms}
                disabled={editing.role === "admin" || !!confirmation}
                onChange={(e) =>
                  update({ all_rooms: e.target.checked, room_ids: [] })
                }
              />
              全场授权（含未匹配设备、未分配选手和未知历史）
            </label>
            {!editing.all_rooms && (
              <fieldset disabled={!!confirmation}>
                <legend>授权机房</legend>
                {snapshot.seating.rooms.map((room) => (
                  <label key={room.id}>
                    <input
                      type="checkbox"
                      checked={editing.room_ids.includes(room.id)}
                      onChange={(e) =>
                        update({
                          room_ids: e.target.checked
                            ? [...editing.room_ids, room.id]
                            : editing.room_ids.filter((id) => id !== room.id),
                        })
                      }
                    />
                    {room.name} · {room.id}
                  </label>
                ))}
              </fieldset>
            )}
            <label>
              {editing.version ? "重置密码（留空保留当前凭据）" : "初始密码"}
              <input
                autoComplete="new-password"
                type={visible ? "text" : "password"}
                minLength={12}
                maxLength={256}
                required={!editing.version}
                disabled={!!confirmation}
                value={password}
                onChange={(e) => {
                  setPassword(e.target.value);
                  setConfirmation(null);
                }}
              />
            </label>
            <div className="actions">
              <button
                type="button"
                disabled={!!confirmation}
                onClick={() => {
                  setPassword(randomSecret());
                  setVisible(true);
                }}
              >
                生成随机密码
              </button>
              <button type="button" onClick={() => setVisible(!visible)}>
                {visible ? "隐藏密码" : "显示密码"}
              </button>
            </div>
            <p>
              请在提交前通过受保护方式保存凭据。关闭窗口或会话失效后清除临时密码。
            </p>
            {confirmation ? (
              <section className="confirmation">
                <h3>核对变更</h3>
                <p>
                  目标 {editing.username} · {editing.id} · 基准 v
                  {editing.version}
                </p>
                <p>
                  当前权限{" "}
                  {before
                    ? `${roleLabels[before.role]} / ${scope(before)} / ${before.enabled ? "启用" : "禁用"}`
                    : "新账号"}
                </p>
                <p>
                  变更后 {roleLabels[editing.role]} / {scope(editing)} /{" "}
                  {editing.enabled ? "启用" : "禁用"}，
                  {password ? "设置新密码" : "沿用当前密码"}。
                </p>
                <p>
                  保存后相关旧会话立即失效。
                  {editing.id === session.id && "本次修改会使你退出登录。"}
                  最后一个启用管理员受到保护。
                </p>
                <code>{String(confirmation.request_id)}</code>
                <div className="actions">
                  <button
                    type="button"
                    className="primary"
                    disabled={disabled}
                    onClick={() => void submit()}
                  >
                    确认保存 / 重试
                  </button>
                  <button
                    type="button"
                    disabled={disabled}
                    onClick={() => setConfirmation(null)}
                  >
                    返回修改
                  </button>
                </div>
              </section>
            ) : (
              <button className="primary" disabled={disabled}>
                预览权限变更
              </button>
            )}
            {error && (
              <p role="alert" className="error">
                {error} · 版本冲突时关闭窗口并重新加载账号。
              </p>
            )}
          </form>
        </DetailsDialog>
      )}
    </section>
  );
}
