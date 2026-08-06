import type { Job } from "../data/jobs";
import { MilestoneRow } from "./MilestoneRow";

export function JobCard({ job }: { job: Job }) {
  const total = job.milestones.reduce((sum, m) => sum + Number(m.amount), 0);
  const paid = job.milestones
    .filter((m) => m.status === "released")
    .reduce((sum, m) => sum + Number(m.amount), 0);

  return (
    <article className="job-card">
      <header className="job-card-header">
        <h2>{job.title}</h2>
        <span className="job-total">
          ${paid} / ${total} {job.token}
        </span>
      </header>
      <p className="job-description">{job.description}</p>
      <div className="job-parties">
        <span>Client: {job.client}</span>
        <span>Freelancer: {job.freelancer}</span>
      </div>
      <ul className="milestone-list">
        {job.milestones.map((milestone) => (
          <MilestoneRow key={milestone.id} jobId={job.id} milestone={milestone} />
        ))}
      </ul>
    </article>
  );
}
