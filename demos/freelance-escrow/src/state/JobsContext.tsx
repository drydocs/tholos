import { useCallback, useMemo, useState, type ReactNode } from "react";
import { jobs as seedJobs, type Job, type Milestone, type MilestoneStatus } from "../data/jobs";
import { assertOutcome, disputeAssertion, finalizeAssertion, resolveAssertion } from "../lib/tholos";
import { JobsContext, type JobsContextValue, type NewJobInput } from "./jobs-context";

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

export function JobsProvider({ children }: { children: ReactNode }) {
  const [jobs, setJobs] = useState<Job[]>(seedJobs);

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
    const assertionId = (await assertOutcome(signerAddress, true)).toString();
    setJobs((current) =>
      updateMilestone(current, jobId, milestoneId, {
        status: "submitted" satisfies MilestoneStatus,
        submittedAt: new Date().toISOString(),
        assertionId,
      }),
    );
  }, []);

  const disputeMilestone = useCallback(async (jobId: string, milestoneId: string, signerAddress: string) => {
    const milestone = findMilestone(jobs, jobId, milestoneId);
    if (!milestone?.assertionId) {
      return;
    }
    await disputeAssertion(signerAddress, BigInt(milestone.assertionId));
    setJobs((current) => updateMilestone(current, jobId, milestoneId, { status: "disputed" }));
  }, [jobs]);

  const voteOnMilestone = useCallback(
    async (jobId: string, milestoneId: string, resolverAddress: string, agreesWithFreelancer: boolean) => {
      const milestone = findMilestone(jobs, jobId, milestoneId);
      if (!milestone?.assertionId) {
        return;
      }
      const decided = await resolveAssertion(resolverAddress, BigInt(milestone.assertionId), agreesWithFreelancer);
      if (decided === null) {
        return;
      }
      setJobs((current) =>
        updateMilestone(current, jobId, milestoneId, {
          status: decided ? "released" : "returned",
        }),
      );
    },
    [jobs],
  );

  const finalizeMilestone = useCallback(async (jobId: string, milestoneId: string, callerAddress: string) => {
    const milestone = findMilestone(jobs, jobId, milestoneId);
    if (!milestone?.assertionId) {
      return;
    }
    await finalizeAssertion(callerAddress, BigInt(milestone.assertionId));
    setJobs((current) => updateMilestone(current, jobId, milestoneId, { status: "released" }));
  }, [jobs]);

  const value = useMemo<JobsContextValue>(
    () => ({
      jobs,
      createJob,
      submitMilestone,
      disputeMilestone,
      voteOnMilestone,
      finalizeMilestone,
    }),
    [jobs, createJob, submitMilestone, disputeMilestone, voteOnMilestone, finalizeMilestone],
  );

  return <JobsContext.Provider value={value}>{children}</JobsContext.Provider>;
}
