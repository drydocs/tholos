import { useState, type ReactNode } from "react";
import { RoleContext, type Role } from "./role-context";

export function RoleProvider({ children }: { children: ReactNode }) {
  const state = useState<Role>("freelancer");
  return <RoleContext.Provider value={state}>{children}</RoleContext.Provider>;
}
