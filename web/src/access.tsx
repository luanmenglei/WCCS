import { createContext, useContext } from "react";
import type { Session } from "./contracts";

export const AccessContext = createContext<Session | null>(null);
export const roleLabels = {
  admin: "管理员",
  operator: "操作员",
  viewer: "只读查看员",
};
export function useAccess() {
  const session = useContext(AccessContext);
  return {
    manage: session?.role === "admin",
    operate: session?.role === "admin" || session?.role === "operator",
    allRooms: session?.all_rooms === true,
  };
}
