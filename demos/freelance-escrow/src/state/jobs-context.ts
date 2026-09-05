import { createContext } from "react";
import type { Job } from "../data/jobs";

export interface NewJobInput {
  title: string;
  description: string;
  client: string;
  freelancer: string;
  token: string;
  milestones: { title: string; amount: string }[];
}

export interface JobsContextValue {
  jobs: Job[];
  createJob: (input: NewJobInput) => void;
  submitMilestone: (jobId: string, milestoneId: string, signerAddress: string) => Promise<void>;
  disputeMilestone: (jobId: string, milestoneId: string, signerAddress: string) => Promise<void>;
  voteOnMilestone: (
    jobId: string,
    milestoneId: string,
    resolverAddress: string,
    agreesWithFreelancer: boolean,
  ) => Promise<void>;
  finalizeMilestone: (jobId: string, milestoneId: string, callerAddress: string) => Promise<void>;
  /**
   * Re-reads the real on-chain assertion state for this milestone and
   * reconciles local status from it, instead of trusting whatever the last
   * optimistic write assumed. Safe to call on any cadence (poll, manual
   * refresh button, or right after an action) — a no-op if the milestone
   * has no assertionId yet. `readAs` only needs to be a connected wallet
   * address; simulation needs a source account but never signs or spends.
   */
  refreshMilestone: (jobId: string, milestoneId: string, readAs: string) => Promise<void>;
}

export const JobsContext = createContext<JobsContextValue | null>(null);
