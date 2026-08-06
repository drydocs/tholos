import { useContext } from "react";
import { JobsContext, type JobsContextValue } from "./jobs-context";

export function useJobs(): JobsContextValue {
  const ctx = useContext(JobsContext);
  if (!ctx) {
    throw new Error("useJobs must be used within a JobsProvider");
  }
  return ctx;
}
