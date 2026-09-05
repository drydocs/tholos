import { useCallback, useMemo, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from "react";
import { jobs as seedJobs, type Job, type Milestone, type MilestoneStatus } from "../data/jobs";
import { JobsContext, type JobsContextValue, type NewJobInput } from "./jobs-context";
import type { Assertion } from "../lib/tholos";

/**
 * lib/tholos.ts pulls in the full Stellar SDK. Importing it dynamically, only
 * at the point a contract call actually happens, keeps that weight out of the
 * initial bundle for the read-only job-browsing path most visits never leave.
 */
function loadTholosClient() {
  return import("../lib/tholos");
}

function updateMilestone(
  jobs: Job[],
  jobId: string,
  milestoneId: string,
  patch: Partial<Milestone>,
): Job[] {
  return jobs.map((job) =>
    job.id !== jobId
      ? job
      : {
          ...job,
          milestones: job.milestones.map((milestone) =>
            milestone.id !== milestoneId ? milestone : { ...milestone, ...patch },
          ),
        },
  );
}

function findMilestone(jobs: Job[], jobId: string, milestoneId: string): Milestone | undefined {
  return jobs.find((job) => job.id === jobId)?.milestones.find((m) => m.id === milestoneId);
}

/**
 * The one place that turns a real `Assertion` read into local milestone
 * state. `status` is Pending/Disputed/Resolved on-chain, never anything
 * about "challenge window elapsed" (the contract doesn't track that as a
 * transition, only `finalize` does), so a still-`Pending` assertion always
 * maps back to `submitted` here regardless of how much time has passed —
 * the "ready to finalize" hint in MilestoneRow is a separate, client-side
 * computation over `opened_at` and is never sourced from `status`.
 */
function mapAssertionToPatch(assertion: Assertion): Partial<Milestone> {
  const assertionOpenedAt = assertion.opened_at.toString();
  if (assertion.status.tag === "Disputed") {
    return { status: "disputed" satisfies MilestoneStatus, assertionOpenedAt };
  }
  if (assertion.status.tag === "Resolved") {
    // `final_outcome` is guaranteed `Some` once `status` is `Resolved` (see
    // docs/src/INTEGRATION.md#reading-the-outcome); `true` means the
    // asserter's original claim stood (the freelancer's "done"), `false`
    // means it didn't.
    return {
      status: (assertion.final_outcome ? "released" : "returned") satisfies MilestoneStatus,
      assertionOpenedAt,
    };
  }
  return { status: "submitted" satisfies MilestoneStatus, assertionOpenedAt };
}

/**
 * Per-JobsProvider bookkeeping shared by every reconcileFromChain call:
 * `counter` hands each call a strictly increasing id the moment it starts
 * (so issue order across concurrent calls — a background poll vs. an
 * action's own reconcile — is always resolvable), and `applied` remembers
 * the highest id actually written to state per milestone, so a result is
 * only ever dropped when a *later-issued* result has *already applied* —
 * never just because another call is merely in flight.
 */
interface ReconcileTracker {
  counter: number;
  applied: Map<string, number>;
}

/**
 * Re-reads real on-chain state for one milestone's assertion and reconciles
 * local status from it. Used both right after an action (instead of trusting
 * a hardcoded guess about what the call must have done) and from background
 * polling / a manual refresh — one code path either way, so a background
 * poll and an action-triggered reconcile can genuinely be in flight for the
 * same milestone at once.
 *
 * Two failure modes this guards against:
 * - Out-of-order responses: `tracker` drops a response whose call was
 *   superseded by a later-issued call that has already applied its result,
 *   so a slow stale poll response can never overwrite a fresher one.
 * - A failed read right after a call whose contract invocation already
 *   returned the real, deterministic outcome (finalize's and resolve's own
 *   return values, not a guess about what they must have done): if the
 *   caller passes `fallbackPatch` built from that value, it's applied
 *   instead of leaving the UI on stale pre-action status with nothing but a
 *   console.warn to show for it.
 */
async function reconcileFromChain(
  setJobs: Dispatch<SetStateAction<Job[]>>,
  tracker: { current: ReconcileTracker },
  jobId: string,
  milestoneId: string,
  assertionId: string,
  readAs: string,
  fallbackPatch?: Partial<Milestone>,
): Promise<void> {
  const key = `${jobId}:${milestoneId}`;
  const mySeq = ++tracker.current.counter;

  function applyIfNewest(patch: Partial<Milestone>) {
    if (mySeq <= (tracker.current.applied.get(key) ?? 0)) {
      return;
    }
    tracker.current.applied.set(key, mySeq);
    setJobs((current) => updateMilestone(current, jobId, milestoneId, patch));
  }

  try {
    const { getAssertionState } = await loadTholosClient();
    const assertion = await getAssertionState(BigInt(assertionId), readAs);
    applyIfNewest(mapAssertionToPatch(assertion));
  } catch (err) {
    console.warn(
      `Could not read back on-chain state for milestone ${milestoneId} (assertion ${assertionId})` +
        (fallbackPatch ? "; applying the already-known result instead." : "; will retry on next refresh."),
      err,
    );
    if (fallbackPatch) {
      applyIfNewest(fallbackPatch);
    }
  }
}

export function JobsProvider({ children }: { children: ReactNode }) {
  const [jobs, setJobs] = useState<Job[]>(seedJobs);
  const reconcileTrackerRef = useRef<ReconcileTracker>({ counter: 0, applied: new Map() });

  const createJob = useCallback((input: NewJobInput) => {
    const jobId = `job-${crypto.randomUUID()}`;
    const job: Job = {
      id: jobId,
      title: input.title,
      description: input.description,
      client: input.client,
      freelancer: input.freelancer,
      token: input.token,
      milestones: input.milestones.map((m, index) => ({
        id: `${jobId}-m${index + 1}`,
        title: m.title,
        amount: m.amount,
        status: "in_progress",
      })),
    };
    setJobs((current) => [job, ...current]);
  }, []);

  const submitMilestone = useCallback(async (jobId: string, milestoneId: string, signerAddress: string) => {
    const { assertOutcome } = await loadTholosClient();
    const assertionId = (await assertOutcome(signerAddress, true)).toString();
    // assert_outcome succeeding guarantees a fresh Pending assertion exists;
    // that much is certain, so it's set immediately rather than waiting on a
    // round-trip. Everything else (and opened_at, needed for the
    // finalize-eligibility hint) comes from a real read right after.
    setJobs((current) =>
      updateMilestone(current, jobId, milestoneId, {
        status: "submitted" satisfies MilestoneStatus,
        submittedAt: new Date().toISOString(),
        assertionId,
      }),
    );
    await reconcileFromChain(setJobs, reconcileTrackerRef, jobId, milestoneId, assertionId, signerAddress);
  }, []);

  const disputeMilestone = useCallback(async (jobId: string, milestoneId: string, signerAddress: string) => {
    const milestone = findMilestone(jobs, jobId, milestoneId);
    if (!milestone?.assertionId) {
      return;
    }
    const { disputeAssertion } = await loadTholosClient();
    await disputeAssertion(signerAddress, BigInt(milestone.assertionId));
    // dispute succeeding guarantees Disputed; reconcile picks up the rest
    // (and corrects this if, improbably, something else changed it first).
    setJobs((current) =>
      updateMilestone(current, jobId, milestoneId, { status: "disputed" satisfies MilestoneStatus }),
    );
    await reconcileFromChain(setJobs, reconcileTrackerRef, jobId, milestoneId, milestone.assertionId, signerAddress);
  }, [jobs]);

  const voteOnMilestone = useCallback(
    async (jobId: string, milestoneId: string, resolverAddress: string, agreesWithFreelancer: boolean) => {
      const milestone = findMilestone(jobs, jobId, milestoneId);
      if (!milestone?.assertionId) {
        return;
      }
      const { resolveAssertion } = await loadTholosClient();
      const decided = await resolveAssertion(resolverAddress, BigInt(milestone.assertionId), agreesWithFreelancer);
      if (decided === null) {
        // Majority not reached yet; still Disputed, nothing to reconcile.
        return;
      }
      // resolve succeeding with a non-null verdict guarantees the same
      // outcome mapping mapAssertionToPatch uses for a Resolved assertion;
      // pass it as the known fallback in case the follow-up read fails.
      await reconcileFromChain(
        setJobs,
        reconcileTrackerRef,
        jobId,
        milestoneId,
        milestone.assertionId,
        resolverAddress,
        { status: (decided ? "released" : "returned") satisfies MilestoneStatus },
      );
    },
    [jobs],
  );

  const finalizeMilestone = useCallback(async (jobId: string, milestoneId: string, callerAddress: string) => {
    const milestone = findMilestone(jobs, jobId, milestoneId);
    if (!milestone?.assertionId) {
      return;
    }
    const { finalizeAssertion } = await loadTholosClient();
    const outcome = await finalizeAssertion(callerAddress, BigInt(milestone.assertionId));
    // finalizeAssertion already returns the contract's own outcome for this
    // assertion (same true/false meaning as mapAssertionToPatch's Resolved
    // case) — use that real result as the known fallback in case the
    // follow-up read fails, instead of assuming what it must have been.
    await reconcileFromChain(
      setJobs,
      reconcileTrackerRef,
      jobId,
      milestoneId,
      milestone.assertionId,
      callerAddress,
      { status: (outcome ? "released" : "returned") satisfies MilestoneStatus },
    );
  }, [jobs]);

  // Kept in sync with `jobs` on every render, but deliberately not a
  // dependency of `refreshMilestone` below: that callback is held in a
  // MilestoneRow's polling-interval effect, and if its identity changed
  // every time *any* milestone's state changed, one milestone's refresh
  // would reset every other actively-polling row's timer.
  const jobsRef = useRef(jobs);
  jobsRef.current = jobs;

  const refreshMilestone = useCallback(async (jobId: string, milestoneId: string, readAs: string) => {
    const milestone = findMilestone(jobsRef.current, jobId, milestoneId);
    if (!milestone?.assertionId) {
      return;
    }
    await reconcileFromChain(setJobs, reconcileTrackerRef, jobId, milestoneId, milestone.assertionId, readAs);
  }, []);

  const value = useMemo<JobsContextValue>(
    () => ({
      jobs,
      createJob,
      submitMilestone,
      disputeMilestone,
      voteOnMilestone,
      finalizeMilestone,
      refreshMilestone,
    }),
    [jobs, createJob, submitMilestone, disputeMilestone, voteOnMilestone, finalizeMilestone, refreshMilestone],
  );

  return <JobsContext.Provider value={value}>{children}</JobsContext.Provider>;
}
