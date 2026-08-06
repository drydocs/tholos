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

export const jobs: Job[] = [
  {
    id: "job-1",
    title: "Landing page redesign",
    description:
      "Redesign the marketing site landing page: new hero, pricing table, and mobile layout.",
    client: "Nova Analytics",
    freelancer: "Priya Chen",
    token: "USDC",
    milestones: [
      {
        id: "job-1-m1",
        title: "Wireframes and content outline",
        amount: "250",
        status: "released",
      },
      {
        id: "job-1-m2",
        title: "Hero and pricing section build",
        amount: "600",
        status: "in_progress",
      },
      {
        id: "job-1-m3",
        title: "Mobile layout and QA pass",
        amount: "400",
        status: "in_progress",
      },
    ],
  },
  {
    id: "job-2",
    title: "Onboarding email sequence",
    description:
      "Write and implement a 5-part onboarding email sequence in the existing ESP template.",
    client: "Fernbank Studio",
    freelancer: "Diego Ramirez",
    token: "USDC",
    milestones: [
      {
        id: "job-2-m1",
        title: "Copy draft for all 5 emails",
        amount: "300",
        status: "in_progress",
      },
      {
        id: "job-2-m2",
        title: "ESP implementation and send test",
        amount: "200",
        status: "in_progress",
      },
    ],
  },
  {
    id: "job-3",
    title: "API rate-limit middleware",
    description:
      "Add a configurable rate-limit middleware to the existing Node API gateway, with tests.",
    client: "Harrow Logistics",
    freelancer: "Amara Okafor",
    token: "USDC",
    milestones: [
      {
        id: "job-3-m1",
        title: "Middleware implementation",
        amount: "500",
        status: "released",
      },
      {
        id: "job-3-m2",
        title: "Load test and tuning",
        amount: "350",
        status: "in_progress",
      },
    ],
  },
];
