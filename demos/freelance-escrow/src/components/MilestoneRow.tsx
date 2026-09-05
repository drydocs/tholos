import { useEffect, useState } from "react";
import type { Milestone } from "../data/jobs";
import { useJobs } from "../state/useJobs";
import { useRole } from "../state/useRole";
import { useWallet } from "../hooks/useWallet";
import { CHALLENGE_WINDOW_SECS } from "../lib/config";

const STATUS_LABEL: Record<Milestone["status"], string> = {
  in_progress: "In progress",
  submitted: "Awaiting review",
  disputed: "Disputed",
  released: "Paid out",
  returned: "Returned to client",
};

/** How often to re-read on-chain state for a milestone that isn't settled yet. */
const POLL_INTERVAL_MS = 30_000;

function isSettled(status: Milestone["status"]): boolean {
  return status === "released" || status === "returned";
}

export function MilestoneRow({ jobId, milestone }: { jobId: string; milestone: Milestone }) {
  const { wallet } = useWallet();
  const [role] = useRole();
  const { submitMilestone, disputeMilestone, voteOnMilestone, finalizeMilestone, refreshMilestone } = useJobs();
  const [busy, setBusy] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const address = wallet.status === "connected" ? wallet.address : null;

  async function run(action: () => Promise<void>) {
    if (!address) {
      setErrorMessage("Connect a wallet first.");
      return;
    }
    setBusy(true);
    setErrorMessage(null);
    try {
      await action();
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : "Something went wrong.");
    } finally {
      setBusy(false);
    }
  }

  const assertionId = milestone.assertionId;
  const settled = isSettled(milestone.status);

  /**
   * Reconcile against real on-chain state on an interval for any milestone
   * that has an assertion and isn't settled yet, so status advances even
   * when nothing happened in this tab: someone else's dispute, vote, or
   * finalize call landing, or a challenge window quietly expiring.
   */
  useEffect(() => {
    if (!address || !assertionId || settled) {
      return;
    }
    const id = setInterval(() => {
      refreshMilestone(jobId, milestone.id, address);
    }, POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, [address, assertionId, settled, jobId, milestone.id, refreshMilestone]);

  // The contract has no getter for its own configured challenge window (see
  // lib/config.ts), so this is a client-side estimate off a real
  // Assertion.opened_at read — a hint, not a gate. The "Finalize and
  // release" call below is always the real gate; the contract rejects it
  // outright if called early.
  const readyToFinalize =
    milestone.status === "submitted" &&
    milestone.assertionOpenedAt !== undefined &&
    Date.now() >= Number(milestone.assertionOpenedAt) * 1000 + CHALLENGE_WINDOW_SECS * 1000;

  return (
    <li className={`milestone milestone--${milestone.status}`}>
      <div className="milestone-main">
        <span className="milestone-title">{milestone.title}</span>
        <span className="milestone-amount">${milestone.amount}</span>
      </div>
      <div className="milestone-meta">
        <span className={`status-badge status-badge--${milestone.status}`}>
          {STATUS_LABEL[milestone.status]}
        </span>
        {milestone.submittedAt && milestone.status === "submitted" && (
          <span className="milestone-submitted">
            submitted {new Date(milestone.submittedAt).toLocaleDateString()}
          </span>
        )}
        {milestone.assertionId && (
          <span className="milestone-assertion">assertion #{milestone.assertionId}</span>
        )}
        {readyToFinalize && <span className="milestone-ready">ready to finalize</span>}
        {assertionId && !settled && (
          <button
            className="button--refresh"
            disabled={busy}
            onClick={() => run(() => refreshMilestone(jobId, milestone.id, address!))}
          >
            Refresh
          </button>
        )}
      </div>

      <div className="milestone-actions">
        {milestone.status === "in_progress" && role === "freelancer" && (
          <button disabled={busy} onClick={() => run(() => submitMilestone(jobId, milestone.id, address!))}>
            Mark milestone done
          </button>
        )}

        {milestone.status === "submitted" && role === "client" && (
          <button
            className="button--danger"
            disabled={busy}
            onClick={() => run(() => disputeMilestone(jobId, milestone.id, address!))}
          >
            Dispute
          </button>
        )}

        {milestone.status === "submitted" && (
          <button disabled={busy} onClick={() => run(() => finalizeMilestone(jobId, milestone.id, address!))}>
            Finalize and release
          </button>
        )}

        {milestone.status === "disputed" && role === "resolver" && (
          <div className="resolver-vote">
            <button disabled={busy} onClick={() => run(() => voteOnMilestone(jobId, milestone.id, address!, true))}>
              Freelancer is right
            </button>
            <button
              className="button--danger"
              disabled={busy}
              onClick={() => run(() => voteOnMilestone(jobId, milestone.id, address!, false))}
            >
              Client is right
            </button>
          </div>
        )}
      </div>

      {errorMessage && <p className="milestone-error">{errorMessage}</p>}
    </li>
  );
}
