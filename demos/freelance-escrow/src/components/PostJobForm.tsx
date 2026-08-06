import { useState } from "react";
import { useJobs } from "../state/useJobs";

interface DraftMilestone {
  title: string;
  amount: string;
}

const EMPTY_MILESTONE: DraftMilestone = { title: "", amount: "" };

export function PostJobForm({ onDone }: { onDone: () => void }) {
  const { createJob } = useJobs();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [client, setClient] = useState("");
  const [freelancer, setFreelancer] = useState("");
  const [milestones, setMilestones] = useState<DraftMilestone[]>([{ ...EMPTY_MILESTONE }]);

  function updateMilestone(index: number, patch: Partial<DraftMilestone>) {
    setMilestones((current) => current.map((m, i) => (i === index ? { ...m, ...patch } : m)));
  }

  function addMilestone() {
    setMilestones((current) => [...current, { ...EMPTY_MILESTONE }]);
  }

  function removeMilestone(index: number) {
    setMilestones((current) => current.filter((_, i) => i !== index));
  }

  const canSubmit =
    title.trim() &&
    client.trim() &&
    freelancer.trim() &&
    milestones.length > 0 &&
    milestones.every((m) => m.title.trim() && Number(m.amount) > 0);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) {
      return;
    }
    createJob({
      title: title.trim(),
      description: description.trim(),
      client: client.trim(),
      freelancer: freelancer.trim(),
      token: "USDC",
      milestones,
    });
    onDone();
  }

  return (
    <form className="post-job-form" onSubmit={handleSubmit}>
      <h2>Post a job</h2>

      <label>
        Job title
        <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Landing page redesign" />
      </label>

      <label>
        Description
        <textarea
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="What does the freelancer need to deliver?"
        />
      </label>

      <div className="post-job-row">
        <label>
          Client name
          <input value={client} onChange={(e) => setClient(e.target.value)} placeholder="Your company" />
        </label>
        <label>
          Freelancer name
          <input value={freelancer} onChange={(e) => setFreelancer(e.target.value)} placeholder="Who you're hiring" />
        </label>
      </div>

      <fieldset className="milestone-fieldset">
        <legend>Milestones</legend>
        {milestones.map((milestone, index) => (
          <div className="milestone-draft-row" key={index}>
            <input
              value={milestone.title}
              onChange={(e) => updateMilestone(index, { title: e.target.value })}
              placeholder="Milestone description"
            />
            <input
              value={milestone.amount}
              onChange={(e) => updateMilestone(index, { amount: e.target.value })}
              placeholder="Amount"
              inputMode="decimal"
            />
            {milestones.length > 1 && (
              <button type="button" className="button--icon" onClick={() => removeMilestone(index)}>
                Remove
              </button>
            )}
          </div>
        ))}
        <button type="button" onClick={addMilestone}>
          Add milestone
        </button>
      </fieldset>

      <div className="post-job-actions">
        <button type="button" onClick={onDone}>
          Cancel
        </button>
        <button type="submit" disabled={!canSubmit}>
          Post job
        </button>
      </div>
    </form>
  );
}
