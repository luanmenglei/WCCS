import { z } from "zod";
import { roomSchema } from "./seating";
export const participantSchema = z.object({
  id: z.string(),
  code: z.string(),
  name: z.string(),
  school: z.string(),
  seat_id: z.string().nullable(),
});
export const contestSchema = z.object({
  id: z.string(),
  name: z.string(),
  version: z.number().int(),
  participants: z.array(participantSchema),
});
export const rosterSchema = z.object({
  version: z.number().int(),
  contests: z.array(contestSchema),
});
export const rosterPreviewSchema = z.object({
  contest_id: z.string(),
  name: z.string(),
  base_version: z.number(),
  preview_token: z.string(),
  total: z.number(),
  rooms: z.array(roomSchema),
  changes: z.array(
    z.object({
      line: z.number(),
      kind: z.enum(["added", "modified", "conflict", "unchanged"]),
      before: participantSchema.nullable(),
      after: participantSchema,
    }),
  ),
  errors: z.array(z.object({ line: z.number(), message: z.string() })),
  skipped: z.array(z.object({ seat_id: z.string(), reason: z.string() })),
});
export type Contest = z.infer<typeof contestSchema>;
export type Participant = z.infer<typeof participantSchema>;
export const changeLabels = {
  added: "新增",
  modified: "修改",
  conflict: "冲突",
  unchanged: "未变化",
};
export const sampleRoster =
  "code\tname\tschool\nS001\t合成选手甲\t合成第一学校\nS002\t合成选手乙\t合成第二学校\nS003\t合成选手丙\t合成第一学校\n";
export function participantLabel(contest: Contest | undefined, seatId: string) {
  const p = contest?.participants.find((p) => p.seat_id === seatId);
  return p ? `${p.name} · ${p.school} · ${p.code} · 已分配` : "未分配选手";
}
