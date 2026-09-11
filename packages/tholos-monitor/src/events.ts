import type { Severity } from "./types.js";

/**
 * Static severity/description for every event name currently emitted by
 * either deployment (see the issue this package closes, #189, and
 * `contracts/tholos/src/lib.rs` / `contracts/tholos-v2/src/lib.rs`).
 *
 * Event names are unique enough across both contracts that one shared table
 * is simpler than two: where v1 and v2 both emit an event with the same
 * name (`PauseUpdated`, `AdminUpdated`, `Disputed`, `Asserted`, `Finalized`,
 * `Resolved`), they mean the same operational thing, so one severity
 * applies to both.
 *
 * `PauseUpdated` is handled separately in `classify()` below: pausing
 * (`paused: true`) is critical, unpausing is not.
 *
 * Deliberately NOT exhaustive-checked against a generated event-name union:
 * a new event a contract upgrade adds shows up here as "unknown" (see
 * `UNKNOWN_EVENT`) rather than crashing or being silently dropped. Add it to
 * this table when it does; until then it's still logged and, by default,
 * treated as `warning` so it isn't missed.
 */
const REGISTRY: Record<string, { severity: Severity; description: string }> = {
  // --- Needs attention right away: admin/governance/emergency actions and
  // liveness failures (the resolver committee failing to do its job in
  // time), matching issue #189's "at minimum" list plus what turned up
  // reading the current contracts (StalledDisputeReclaimed, RoundVoided —
  // neither was in the issue's event list, added upstream after it was
  // filed; see the package README for why they're included anyway).
  AdminUpdated: {
    severity: "critical",
    description: "Contract admin key was rotated.",
  },
  AdminRotationProposed: {
    severity: "critical",
    description: "A new admin rotation was proposed (v1) and is pending.",
  },
  RotationCancelled: {
    severity: "critical",
    description:
      "A resolver rotation proposal was cancelled — often the deadlock guard firing because it could no longer reach a majority.",
  },
  RoundCancelled: {
    severity: "critical",
    description: "v2 admin emergency-cancelled an active round.",
  },
  RoundVoided: {
    severity: "critical",
    description:
      "v2 round voided: revealed weight didn't clear the quorum floor, so no outcome could be locked.",
  },
  StalledDisputeReclaimed: {
    severity: "critical",
    description:
      "A dispute stalled long enough (resolver committee didn't act in time) that both bonds were reclaimed with no winner.",
  },

  // --- Governance/parameter changes: worth knowing, rarely an emergency.
  BondAmountUpdated: {
    severity: "warning",
    description: "Bond amount parameter changed.",
  },
  ResolversUpdated: {
    severity: "warning",
    description: "Resolver committee was replaced (admin override path).",
  },
  RotationProposed: {
    severity: "warning",
    description: "A resolver self-rotation was proposed.",
  },
  RotationExecuted: {
    severity: "warning",
    description: "A resolver self-rotation executed.",
  },
  StallTimeoutUpdated: {
    severity: "warning",
    description: "Stalled-dispute reclaim timeout parameter changed.",
  },

  // --- Ordinary protocol lifecycle: expected traffic, not incidents.
  Asserted: { severity: "info", description: "New assertion posted." },
  Disputed: { severity: "info", description: "An assertion was disputed." },
  Finalized: {
    severity: "info",
    description: "An uncontested assertion finalized.",
  },
  Resolved: {
    severity: "info",
    description: "A disputed assertion was resolved by the committee.",
  },
  PositionFunded: { severity: "info", description: "v2 position funded." },
  RevealOpened: {
    severity: "info",
    description: "v2 reveal phase opened for a round.",
  },
  Revealed: { severity: "info", description: "v2 vote revealed." },
  Settled: { severity: "info", description: "v2 position settled." },
  DustCredited: {
    severity: "info",
    description: "v2 leftover dust credited to a settled position.",
  },
  Withdrawn: { severity: "info", description: "v2 credit balance withdrawn." },
};

const UNKNOWN_EVENT = {
  severity: "warning" as Severity,
  description:
    "Event name not in this service's registry — likely a contract upgrade added it. Update packages/tholos-monitor/src/events.ts.",
};

/**
 * `PauseUpdated` is the one event whose severity depends on its payload, not
 * just its name: pausing is an incident (everything user-facing on that
 * deployment just stopped), unpausing is the incident ending. `data` is
 * already `scValToNative`-decoded by the time this runs (see rpc.ts), so a
 * boolean `paused` field is all that's expected here.
 */
function hasPausedField(data: unknown): data is { paused: unknown } {
  return typeof data === "object" && data !== null && "paused" in data;
}

function classifyPauseUpdated(data: unknown): {
  severity: Severity;
  description: string;
} {
  const paused = hasPausedField(data) ? data.paused : undefined;
  if (paused === true) {
    return { severity: "critical", description: "Contract paused." };
  }
  if (paused === false) {
    return { severity: "info", description: "Contract unpaused." };
  }
  // Shouldn't happen against the current contracts, but never let a
  // decoding surprise downgrade a pause event into silence.
  return {
    severity: "critical",
    description:
      "PauseUpdated event with an unrecognized payload shape — treating as critical rather than risk missing a real pause.",
  };
}

export function classify(
  eventName: string,
  data: unknown,
): { severity: Severity; description: string } {
  if (eventName === "PauseUpdated") {
    return classifyPauseUpdated(data);
  }
  return REGISTRY[eventName] ?? UNKNOWN_EVENT;
}
