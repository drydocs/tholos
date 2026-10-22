export type MilestoneStatus =
  | "in_progress"
  | "submitted"
  | "disputed"
  | "released"
  | "returned";

export interface Milestone {
  id: string;
  title: string;
  amount: string;
  status: MilestoneStatus;
  /** Set once the freelancer has submitted; drives the challenge-window countdown. */
  submittedAt?: string;
  /**
   * The Tholos assertion id backing this milestone, once assert_outcome has
   * been called. Job/milestone metadata itself never touches the contract;
   * this is the one piece of state that maps this app's data to Tholos's,
   * kept client-side per the pattern in docs/src/INTEGRATION.md.
   */
  assertionId?: string;
  /**
   * `Assertion.opened_at` (ledger timestamp, seconds) from the most recent
   * `get_assertion_state` read. Used only to derive a "review window has
   * likely closed" hint client-side (see VITE_CHALLENGE_WINDOW_SECS in
   * lib/config.ts) since the contract exposes no getter for the configured
   * challenge window itself. Never authoritative for whether `finalize`
   * will actually succeed — the contract is.
   */
  assertionOpenedAt?: string;
}

export interface Job {
  id: string;
  title: string;
  description: string;
  client: string;
  freelancer: string;
  token: string;
  milestones: Milestone[];
}
