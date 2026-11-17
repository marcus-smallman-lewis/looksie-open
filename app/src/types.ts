// Mirrors crates/looksie-protocol. Field names are camelCase on the wire.

export type PrRef = { host: string; owner: string; repo: string; number: number };

export type PrSummary = {
  pr: PrRef;
  headSha?: string;
  title: string;
  author: string;
  agent: boolean;
  private: boolean;
  additions: number;
  deletions: number;
  changedFiles: number;
  updatedAt: string;
  url: string;
  relation: string;
  links?: string[];
};

export type GroupSuggestion = { prs: PrRef[]; name: string; reasons: string[] };
export type Shop = {
  status: { plan: "free" | "trial" | "pro"; trialDaysLeft: number | null; trialUsed: boolean; keyHint: string | null; checkOverdue?: boolean };
  priceMonthly: string;
  priceYearly: string;
  offer: string;
  checkoutMonthly: string;
  checkoutYearly: string;
  portal: string;
  trialDays: number;
};

export type PrDetail = {
  summary: PrSummary;
  body: string;
  headSha: string;
  baseSha: string;
  baseRef: string;
  headRef: string;
  defaultBranch: string;
  state: string;
  commits: string[];
  linkedIssues: { number: number; title: string; body: string; url: string }[];
  ci: { total: number; failed: number; pending: number; unknown?: boolean };
};

export type LineKind = "add" | "del" | "ctx";
export type DiffLine = { kind: LineKind; oldNo: number | null; newNo: number | null; text: string };
export type Hunk = { header: string; lines: DiffLine[] };

export type FileChange = {
  pr: PrRef;
  path: string;
  previousPath: string | null;
  kind: "added" | "modified" | "removed" | "renamed";
  additions: number;
  deletions: number;
  patch: string | null;
  hunks: Hunk[];
  url: string;
  trimmed?: number;
};

export type Thread = { pr: PrRef; path: string; line: number | null; author: string; body: string; url: string };

export type Layer = "data" | "server" | "screens" | "tests-and-build" | "docs";
export type Flag = "none" | "look" | "problem";
export type Fact = { text: string; source: string };
export type StepFile = { pr: PrRef; path: string; additions: number; deletions: number; focus: string[] | null };

export type Step = {
  id: string;
  title: string;
  layer: Layer;
  tier: "decide" | "glance";
  score: number;
  reasons: string[];
  files: StepFile[];
  facts: Fact[];
  question: string | null;
  flag: Flag;
  buildsOn: string[];
  leadsTo: string[];
  ai: { question: string; why: string; flag: Flag } | null;
  /** Scored like a decision, but past the limit for a review this size. Waits first among the quick looks. */
  overLimit?: boolean;
};

export type ClaimStatus = "backed" | "contradicted" | "not-found" | "unchecked";
export type Claim = { pr: PrRef; text: string; status: ClaimStatus; note: string; stepId: string | null; source: string };

export type Briefing = {
  title: string;
  headline?: string | null;
  descriptions: { pr: PrRef; text: string }[];
  aiSummary: string | null;
  claims: Claim[];
  needsYou: Fact[];
  notMentioned: Fact[];
  fits: Fact[];
  sources: string[];
};

export type SkippedFile = { pr: PrRef; path: string; reason: string; lines: number };

export type Review = {
  id: string;
  prs: PrDetail[];
  briefing: Briefing;
  steps: Step[];
  files: FileChange[];
  skipped: SkippedFile[];
  threads: Thread[];
};

export type Verdict = "approve" | "request-changes" | "comment";
export type DraftComment = { anchor: { path: string; side: "old" | "new"; startLine: number; endLine: number } | null; body: string };
export type ReviewSubmission = { pr: PrRef; commit: string; verdict: Verdict; summary: string; comments: DraftComment[] };

export type AuthStatus = { login: string; source: string };
export type AiModel = { id: string; name: string; note: string; provider: string };
/** A company whose API key can be used: Anthropic or OpenAI. */
export type AiProvider = { id: string; name: string; assistant: string; hasKey: boolean; keyHint: string | null; keyStart: string; keysUrl: string; keysPage: string };
/** How AI is set up: the reviewer's own API keys, the chosen model (and so company), and the commands that connect their own Claude Code or Codex. */
export type AiSetup = { hasKey: boolean; keyHint: string | null; model: string; provider: string; providers: AiProvider[]; models: AiModel[] };
/** The reviewer's own Claude Code or Codex. `command` is what they type there to start a review; `manual` is connecting by hand. */
export type AgentState = { id: "claude" | "codex"; name: string; found: boolean; connected: boolean; elsewhere: boolean; command: string; manual: string[] };
export type SavedAi = { key: string; layer: AiLayer; savedAt: number };
export type Account = { id: string; host: string; login: string; source: "cli" | "saved" | "env"; sourceText: string; working: boolean; problem: string | null; active: boolean };
export type CachedReview = { heads: string[]; builtAt: number };
export type AiLayer = {
  provider: string;
  summary: string | null;
  steps: { id: string; question: string; why: string; flag: Flag }[];
  claims: { index: number; status: ClaimStatus; note: string }[];
  dropped: number;
};

// What the reviewer has done so far. Saved on this computer only.
export type Answer = "fine" | "change" | "question";
export type Saved = {
  answers: Record<string, Answer | undefined>;
  notes: Record<string, string>;
  comments: Record<string, string>;
  verdicts: Record<string, Verdict | undefined>;
  summary: string;
};
/** Saved, as shared on disk with the reviewer's own agent (looksie_core::drafts::Draft). */
export type Posted = { url: string; commit: string; at: number };
export type Draft = { key: string; answers: Record<string, string>; notes: Record<string, string>; comments: Record<string, string>; verdicts: Record<string, string>; summary: string; posted: Record<string, Posted>; updatedAt: number };
/** Only what changed; null removes an entry (looksie_core::drafts::Patch). */
export type DraftPatch = { answers?: Record<string, string | null>; notes?: Record<string, string | null>; comments?: Record<string, string | null>; verdicts?: Record<string, string | null>; summary?: string | null ; posted?: Record<string, Posted | null> };

export const LAYERS: { id: Layer; title: string }[] = [
  { id: "data", title: "Data" },
  { id: "server", title: "Server" },
  { id: "screens", title: "Screens" },
  { id: "tests-and-build", title: "Tests and build" },
  { id: "docs", title: "Docs" },
];

export const prKey = (p: PrRef) => `${p.owner}/${p.repo}#${p.number}`;
export const prLabel = (p: PrRef) => `${p.owner}/${p.repo} #${p.number}`;
export const prShort = (p: PrRef) => `${p.repo} #${p.number}`;
export const samePr = (a: PrRef, b: PrRef) => a.owner === b.owner && a.repo === b.repo && a.number === b.number;
export const hostName = (host: string) => ({ github: "GitHub", "azure-devops": "Azure DevOps", gitlab: "GitLab", bitbucket: "Bitbucket" })[host] ?? "the host";
export const plural = (n: number, word: string) => `${n.toLocaleString("en-US")} ${word}${n === 1 ? "" : "s"}`;
