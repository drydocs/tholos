import { useState } from "react";
import "./App.css";
import { JobCard } from "./components/JobCard";
import { PostJobForm } from "./components/PostJobForm";
import { RoleSwitcher } from "./components/RoleSwitcher";
import { WalletButton } from "./components/WalletButton";
import { JobsProvider } from "./state/JobsContext";
import { useJobs } from "./state/useJobs";
import { RoleProvider } from "./state/RoleContext";

function AppShell() {
  const { jobs } = useJobs();
  const [showPostJob, setShowPostJob] = useState(false);

  return (
    <div className="app">
      <header className="app-header">
        <div className="brand">
          <span className="brand-name">Freelance Milestone Escrow</span>
          <span className="brand-tagline">Post a job, pay by milestone, dispute if something's wrong</span>
        </div>
        <div className="app-header-controls">
          <RoleSwitcher />
          <WalletButton />
        </div>
      </header>

      <main className="app-main">
        <div className="jobs-toolbar">
          <h1>Open jobs</h1>
          <button onClick={() => setShowPostJob(true)}>Post a job</button>
        </div>

        {showPostJob && <PostJobForm onDone={() => setShowPostJob(false)} />}

        <div className="job-list">
          {jobs.map((job) => (
            <JobCard key={job.id} job={job} />
          ))}
        </div>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <RoleProvider>
      <JobsProvider>
        <AppShell />
      </JobsProvider>
    </RoleProvider>
  );
}
