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
}

export const JobsContext = createContext<JobsContextValue | null>(null);
