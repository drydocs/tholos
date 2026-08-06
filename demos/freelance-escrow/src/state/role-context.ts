import { createContext } from "react";

/**
 * A real deployment would derive role from who's logged in. This demo has no
 * backend or auth of its own (Tholos itself doesn't know "client" or
 * "freelancer", only addresses), so the connected wallet plays whichever
 * role you pick here to exercise all three sides of a dispute.
 */
export type Role = "freelancer" | "client" | "resolver";

export const RoleContext = createContext<[Role, (role: Role) => void] | null>(null);
