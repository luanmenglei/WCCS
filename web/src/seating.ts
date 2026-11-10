import { z } from "zod";

export const seatSchema = z.object({
  id: z.string(),
  room_id: z.string(),
  row: z.number().int(),
  column: z.number().int(),
  number: z.number().int(),
  expected_ip: z.string().nullable(),
  status: z.enum(["online", "offline", "unbound", "conflict"]),
  devices: z.array(z.string()),
});
export const roomSchema = z.object({
  id: z.string(),
  name: z.string(),
  rows: z.number().int(),
  columns: z.number().int(),
  start_number: z.number().int(),
  numbering: z.enum(["row", "column", "snake", "custom"]),
  origin: z.enum(["top-left", "top-right", "bottom-left", "bottom-right"]),
  version: z.number().int(),
  seats: z.array(seatSchema),
});
export const seatingSchema = z.object({
  rooms: z.array(roomSchema),
  devices: z.array(
    z.object({
      id: z.string(),
      peer_ip: z.string().nullable(),
      observed_ip: z.string().nullable(),
      address_source: z.string().nullable(),
      reported_ips: z.array(z.string()),
      previous_ip: z.string().nullable(),
      address_version: z.number().int(),
      needs_review: z.boolean(),
      excluded: z.boolean(),
      registered: z.boolean(),
      online: z.boolean(),
    }),
  ),
});
export const roomPreviewSchema = z.object({
  preview_token: z.string(),
  changes: z.array(
    z.object({
      row: z.number(),
      column: z.number(),
      number: z.number(),
      expected_ip: z.string().nullable(),
      kind: z.enum(["added", "retained", "removed"]),
      previous: seatSchema.nullable(),
      incoming_devices: z.array(
        z.object({
          id: z.string(),
          needs_review: z.boolean(),
          online: z.boolean(),
        }),
      ),
    }),
  ),
});
export const targetPreviewSchema = z.object({
  preview_token: z.string(),
  targets: z.array(
    z.object({
      seat_id: z.string(),
      number: z.number(),
      device_id: z.string().nullable(),
      device_ids: z.array(z.string()),
      status: z.enum([
        "ready",
        "unregistered",
        "offline",
        "conflict",
        "outside_configuration",
      ]),
    }),
  ),
});
export type Room = z.infer<typeof roomSchema>;
export type Seat = z.infer<typeof seatSchema>;
export type DraftSeat = Pick<Seat, "row" | "column" | "number" | "expected_ip">;
export type Draft = Omit<Room, "seats"> & { seats: DraftSeat[] };

export function newRoom(): Draft {
  return {
    id: crypto.randomUUID(),
    name: "",
    rows: 3,
    columns: 4,
    start_number: 1,
    numbering: "row",
    origin: "top-left",
    version: 0,
    seats: [],
  };
}

export function layout(draft: Draft): DraftSeat[] {
  if (
    !Number.isInteger(draft.rows) ||
    !Number.isInteger(draft.columns) ||
    draft.rows < 1 ||
    draft.columns < 1 ||
    draft.rows > 50 ||
    draft.columns > 50 ||
    draft.rows * draft.columns > 500
  )
    throw new Error("行列为 1～50 的整数，每间最多 500 个座位");
  const positions: [number, number][] = [];
  const byColumn = draft.numbering === "column";
  const major = byColumn ? draft.columns : draft.rows;
  const minor = byColumn ? draft.rows : draft.columns;
  for (let a = 0; a < major; a++) {
    for (let b = 0; b < minor; b++) {
      const offset = draft.numbering === "snake" && a % 2 ? minor - b - 1 : b;
      let row = byColumn ? offset : a;
      let column = byColumn ? a : offset;
      if (draft.origin.startsWith("bottom")) row = draft.rows - row - 1;
      if (draft.origin.endsWith("right")) column = draft.columns - column - 1;
      positions.push([row, column]);
    }
  }
  const old = new Map(
    draft.seats.map((seat) => [`${seat.row},${seat.column}`, seat]),
  );
  return positions
    .map(([row, column], index) => {
      const previous = old.get(`${row},${column}`);
      return {
        row,
        column,
        number:
          draft.numbering === "custom" && previous
            ? previous.number
            : draft.start_number + index,
        expected_ip: previous?.expected_ip ?? null,
      };
    })
    .sort((a, b) => a.row - b.row || a.column - b.column);
}

export function ipv4(value: string): number {
  const parts = value.split(".");
  if (
    parts.length !== 4 ||
    parts.some((part) => !/^(0|[1-9]\d{0,2})$/.test(part) || Number(part) > 255)
  )
    throw new Error(`IPv4 格式错误 ${value}`);
  return parts.reduce((value, part) => value * 256 + Number(part), 0);
}

export function fillIps(
  seats: DraftSeat[],
  start: string,
  step: number,
  order: string,
): DraftSeat[] {
  if (!Number.isSafeInteger(step) || step < 1)
    throw new Error("步长必须为正整数");
  const first = ipv4(start);
  const sorted = [...seats].sort(
    order === "number"
      ? (a, b) => a.number - b.number
      : (a, b) => a.row - b.row || a.column - b.column,
  );
  const values = new Map(
    sorted.map((seat, index) => {
      const address = first + index * step;
      if (!Number.isSafeInteger(address) || address > 0xffffffff)
        throw new Error("生成地址超出 IPv4 范围");
      return [
        seat,
        [24, 16, 8, 0]
          .map((shift) => Math.floor(address / 2 ** shift) % 256)
          .join("."),
      ];
    }),
  );
  return seats.map((seat) => ({ ...seat, expected_ip: values.get(seat)! }));
}

export function pasteIps(
  draft: Draft,
  text: string,
  mode: string,
): DraftSeat[] {
  const lines = text
    .replace(/\r/g, "")
    .replace(/\n$/, "")
    .split("\n")
    .map((line) => line.split("\t"));
  const mapping = new Map<string, string | null>();
  function add(seat: DraftSeat | undefined, value: string) {
    if (!seat) throw new Error("粘贴中包含未知座位号");
    const key = `${seat.row},${seat.column}`;
    if (mapping.has(key)) throw new Error(`座位号 ${seat.number} 重复`);
    const ip = value.trim() || null;
    if (ip) ipv4(ip);
    mapping.set(key, ip);
  }
  if (mode === "matrix") {
    if (
      lines.length !== draft.rows ||
      lines.some((line) => line.length !== draft.columns)
    )
      throw new Error("IP 矩阵行列数量与机房不符，使用制表符分列");
    lines.forEach((line, row) =>
      line.forEach((ip, column) =>
        add(
          draft.seats.find(
            (seat) => seat.row === row && seat.column === column,
          ),
          ip,
        ),
      ),
    );
  } else {
    for (const line of lines) {
      if (line.length !== 2 || !/^\d+$/.test(line[0].trim()))
        throw new Error("每行需要座位号和 IP 两列，使用制表符分列");
      add(
        draft.seats.find((seat) => seat.number === Number(line[0])),
        line[1],
      );
    }
  }
  return draft.seats.map((seat) => {
    const key = `${seat.row},${seat.column}`;
    return mapping.has(key)
      ? { ...seat, expected_ip: mapping.get(key)! }
      : seat;
  });
}

export function errors(draft: Draft): Map<string, string> {
  const result = new Map<string, string>();
  const generated = draft.numbering === "custom" ? [] : layout(draft);
  const counts = new Map<number, number>();
  const ips = new Map<string, number>();
  for (const seat of draft.seats) {
    counts.set(seat.number, (counts.get(seat.number) ?? 0) + 1);
    if (seat.expected_ip)
      ips.set(seat.expected_ip, (ips.get(seat.expected_ip) ?? 0) + 1);
  }
  for (const seat of draft.seats) {
    const key = `${seat.row},${seat.column}`;
    const expected = generated.find(
      (item) => item.row === seat.row && item.column === seat.column,
    );
    if (expected && expected.number !== seat.number)
      result.set(key, "请应用编号规则");
    if (
      !Number.isInteger(seat.number) ||
      seat.number < draft.start_number ||
      seat.number >= draft.start_number + draft.rows * draft.columns ||
      counts.get(seat.number)! > 1
    )
      result.set(key, "编号重复或超出连续范围");
    if (seat.expected_ip) {
      try {
        ipv4(seat.expected_ip);
      } catch {
        result.set(key, "IPv4 格式错误");
      }
      if (ips.get(seat.expected_ip)! > 1) result.set(key, "IP 地址重复");
    }
  }
  return result;
}
