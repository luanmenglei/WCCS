import { useState } from "react";
import type { Page } from "./navigation";
import type { Session, Snapshot } from "./contracts";
import type { Mutation } from "./SeatChecks";
import { ContestControl } from "./ContestControl";
import { Distribution } from "./Distribution";
import { Domjudge } from "./Domjudge";
import { Events } from "./Events";
import { Maintenance } from "./Maintenance";
import { Operators } from "./Operators";
import { Presentation } from "./Presentation";
import { Recovery } from "./Recovery";
import { Rooms } from "./Rooms";
import { Roster } from "./Roster";
import { Seats } from "./Seats";
import { ConfigurationEditor } from "./ConfigurationEditor";
import { Devices } from "./Devices";
import { Waiting } from "./Waiting";
import { Enrollment } from "./Lifecycle";

type Props = {
  page: Page;
  snapshot: Snapshot;
  session: Session;
  contestId: string;
  seatSelection: string[];
  onSelect: (ids: string[]) => void;
  disabled: boolean;
  onMutate: Mutation;
  onReloadConfiguration: () => Promise<boolean>;
};

export function WorkspacePages({
  page,
  snapshot,
  session,
  contestId,
  seatSelection,
  onSelect,
  disabled,
  onMutate,
  onReloadConfiguration,
}: Props) {
  const [editorKey, setEditorKey] = useState(0);
  const contest = snapshot.roster.contests.find(
    (item) => item.id === contestId,
  );
  const operationsDisabled = disabled || session.role === "viewer";
  const managementDisabled = disabled || session.role !== "admin";
  return (
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
            onMutate={onMutate}
            disabled={disabled}
          />
        ) : (
          <p role="alert">操作员管理需要管理员权限。</p>
        ))}
      {page === "events" && (
        <Events csrf={session.csrf} disabled={operationsDisabled} />
      )}
      {page === "recovery" && (
        <Recovery
          key={contestId}
          snapshot={snapshot}
          contestId={contestId}
          csrf={session.csrf}
          disabled={operationsDisabled}
          onMutate={onMutate}
        />
      )}
      {page === "distribution" && (
        <Distribution
          snapshot={snapshot}
          csrf={session.csrf}
          disabled={disabled}
          onMutate={onMutate}
        />
      )}
      {page === "contest-control" && (
        <ContestControl
          key={contestId}
          snapshot={snapshot}
          contestId={contestId}
          csrf={session.csrf}
          disabled={disabled}
          onMutate={onMutate}
        />
      )}
      {page === "domjudge" && (
        <Domjudge
          key={contestId}
          snapshot={snapshot}
          contestId={contestId}
          csrf={session.csrf}
          disabled={disabled}
          onMutate={onMutate}
        />
      )}
      {page === "presentation" && (
        <Presentation
          key={contestId}
          contestId={contestId}
          csrf={session.csrf}
          disabled={managementDisabled || !session.all_rooms}
          onMutate={onMutate}
        />
      )}
      {page === "maintenance" && (
        <Maintenance
          snapshot={snapshot}
          csrf={session.csrf}
          disabled={operationsDisabled}
          onMutate={onMutate}
        />
      )}
      {(page === "waiting" || page === "login") && (
        <Waiting
          mode={page}
          key={`${page}:${contestId}`}
          snapshot={snapshot}
          contestId={contestId}
          selected={seatSelection}
          onSelect={onSelect}
          csrf={session.csrf}
          disabled={operationsDisabled}
          onMutate={onMutate}
        />
      )}
      {/* Keep these views mounted so navigation preserves drafts and selections. */}
      <div className="page-view" hidden={page !== "roster"}>
        <Roster
          key={contestId}
          snapshot={snapshot}
          contest={contest}
          csrf={session.csrf}
          disabled={managementDisabled}
          onMutate={onMutate}
        />
      </div>
      {(["seats", "targets"] as const).map((view) => (
        <div className="page-view" key={view} hidden={page !== view}>
          <Seats
            targetMode={view === "targets"}
            contest={contest}
            selected={seatSelection}
            onSelect={onSelect}
            snapshot={snapshot}
            csrf={session.csrf}
            disabled={operationsDisabled}
            onMutate={onMutate}
          />
        </div>
      ))}
      <div className="page-view" hidden={page !== "rooms"}>
        <Rooms
          contestId={contestId}
          snapshot={snapshot}
          csrf={session.csrf}
          disabled={managementDisabled}
          onMutate={onMutate}
        />
      </div>
      <div className="page-view" hidden={page !== "configuration"}>
        <ConfigurationEditor
          key={editorKey}
          snapshot={snapshot}
          disabled={managementDisabled}
          onReload={() => {
            void onReloadConfiguration().then((loaded) => {
              if (loaded) setEditorKey((key) => key + 1);
            });
          }}
          onPublish={async (body) => {
            await onMutate(
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
          disabled={managementDisabled}
          onMutate={onMutate}
        />
      </div>
      <div className="page-view" hidden={page !== "devices"}>
        <Devices
          contest={contest}
          onMutate={onMutate}
          snapshot={snapshot}
          disabled={operationsDisabled}
          onCreate={async (body) => {
            await onMutate(
              "devices",
              body,
              "设备已创建，请将注册材料用于 Windows 检查入口。",
            );
          }}
          onRecheck={async (id) => {
            await onMutate(
              "recheck",
              { base_version: snapshot.version, targets: [id] },
              "重新检查任务已下发。",
            );
          }}
        />
      </div>
    </>
  );
}
