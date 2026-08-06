import { useRole } from "../state/useRole";
import type { Role } from "../state/role-context";

const ROLES: { value: Role; label: string }[] = [
  { value: "freelancer", label: "Freelancer" },
  { value: "client", label: "Client" },
  { value: "resolver", label: "Resolver" },
];

export function RoleSwitcher() {
  const [role, setRole] = useRole();
  return (
    <label className="role-switcher">
      Viewing as
      <select value={role} onChange={(e) => setRole(e.target.value as Role)}>
        {ROLES.map((r) => (
          <option key={r.value} value={r.value}>
            {r.label}
          </option>
        ))}
      </select>
    </label>
  );
}
