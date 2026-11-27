import { useEffect, useState } from "react";
import type { Mutation } from "./api";
import type { Snapshot } from "./contracts";
import { layout, newRoom, type Draft } from "./seating";
import { RoomEditor } from "./rooms/RoomEditor";
import { DeviceAddressReview } from "./rooms/DeviceAddressReview";

type Props = {
  snapshot: Snapshot;
  contestId?: string;
  csrf: string;
  disabled: boolean;
  onMutate: Mutation;
};

export function Rooms(props: Props) {
  const [draft, setDraft] = useState<Draft>(() => {
    const room = newRoom();
    return { ...room, seats: layout(room) };
  });
  const [key, setKey] = useState(0);
  const [open, setOpen] = useState(false);
  const [dirty, setDirty] = useState(false);
  useEffect(() => {
    if (!dirty) return;
    const guard = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener("beforeunload", guard);
    return () => window.removeEventListener("beforeunload", guard);
  }, [dirty]);
  function load(room: Draft) {
    if (dirty && !window.confirm("当前机房有未保存的修改。放弃草稿并切换？"))
      return;
    setDirty(false);
    setDraft(room);
    setKey(key + 1);
    setOpen(true);
  }
  return (
    <section id="rooms" className="panel rooms-panel">
      <div className="section-header">
        <div>
          <h2>机房与座位</h2>
          <p>按物理位置维护座位，认证设备通过来源 IP 匹配。</p>
        </div>
        <button
          disabled={props.disabled}
          onClick={() => {
            const room = newRoom();
            load({ ...room, seats: layout(room) });
          }}
        >
          创建机房
        </button>
      </div>
      <div className="room-tabs">
        {props.snapshot.seating.rooms.map((room) => (
          <button
            key={room.id}
            aria-pressed={open && draft.id === room.id}
            onClick={() => load(room)}
          >
            {room.name} · {room.rows} × {room.columns} · v{room.version}
          </button>
        ))}
      </div>
      {open && (
        <RoomEditor
          key={key}
          snapshot={props.snapshot}
          disabled={props.disabled}
          csrf={props.csrf}
          onMutate={props.onMutate}
          initial={draft}
          onDirty={setDirty}
          onReload={() => {
            const room = props.snapshot.seating.rooms.find(
              (room) => room.id === draft.id,
            );
            if (room) load(room);
          }}
        />
      )}
      <DeviceAddressReview
        snapshot={props.snapshot}
        contestId={props.contestId}
        disabled={props.disabled}
        onMutate={props.onMutate}
      />
    </section>
  );
}
