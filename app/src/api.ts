// The only place the UI talks to the backend.
// Inside the app this calls Rust commands. In a plain browser (npm run dev) it reads app/fixtures/*.json instead.

import type { Account, AgentState, AiLayer, AiSetup, Draft, DraftPatch, SavedAi, CachedReview, GroupSuggestion, PrSummary, Review, ReviewSubmission, Shop } from "./types";

/** The same key as ai_store::key in Rust: host/owner/repo#number@headsha, lower case, sorted, joined by +. */
export const reviewKey = (review: Review) => review.prs.map((p) => `${p.summary.pr.host}/${p.summary.pr.owner}/${p.summary.pr.repo}#${p.summary.pr.number}@${p.headSha}`.toLowerCase()).sort().join("+");

const inApp = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (inApp) {
    const { invoke } = await import("@tauri-apps/api/core");
    try {
      return await invoke<T>(command, args);
    } catch (e) {
      throw new Error(typeof e === "string" ? e : String((e as Error)?.message ?? e));
    }
  }
  const res = await fetch(`/fixtures/${command}.json`);
  if (!res.ok) throw new Error(`No fixture for ${command}. Run the app, or add app/fixtures/${command}.json.`);
  return (await res.json()) as T;
}

/** A newer version that has been downloaded and is ready to install. */
export type ReadyUpdate = { version: string; notes: string; install: () => Promise<void> };

export const api = {
  inApp,
  /** Makes the window's own title bar light or dark, to match the app. */
  setWindowTheme: async (dark: boolean) => {
    if (!inApp) return;
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();
    await win.setTheme(dark ? "dark" : "light");
    // On a Mac the title bar is transparent, so painting the window's background is what makes it match.
    await win.setBackgroundColor(dark ? "#191715" : "#f6f3ee");
  },
  version: async () => (inApp ? (await import("@tauri-apps/api/app")).getVersion() : "dev"),
  /** Looks for a newer Looksie. If there is one, downloads it in the background and hands back a way to install it. */
  findUpdate: async (): Promise<ReadyUpdate | null> => {
    if (!inApp) return null;
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) return null;
    await update.download();
    return {
      version: update.version,
      notes: update.body ?? "",
      install: async () => {
        await update.install();
        const { relaunch } = await import("@tauri-apps/plugin-process");
        await relaunch();
      },
    };
  },
  listAccounts: () => call<Account[]>("list_accounts"),
  useAccount: (id: string) => call<Account[]>("use_account", { id }),
  addAccount: (token: string, host: string) => call<Account[]>("add_account", { token, host }),
  removeAccount: (id: string) => call<Account[]>("remove_account", { id }),
  cachedReviews: () => call<Record<string, CachedReview>>("cached_reviews"),
  listInbox: () => call<PrSummary[]>("list_inbox"),
  suggestGroups: (prs: PrSummary[]) => call<GroupSuggestion[]>("suggest_groups", { prs }),
  shop: () => call<Shop>("licence_status"),
  recheckLicence: () => (inApp ? call<Shop>("recheck_licence") : call<Shop>("licence_status")),
  offerLeft: () => call<{ left: number; max: number } | null>("offer_left"),
  startTrial: () => call<Shop>("start_trial"),
  activateLicence: (key: string) => call<Shop>("activate_licence", { key }),
  removeLicence: () => call<Shop>("remove_licence"),
  buildReview: (prs: string[], groupName?: string, heads?: string[], fresh?: boolean) => call<Review>("build_review", { prs, groupName: groupName ?? null, heads: heads ?? null, fresh: !!fresh }),
  reviewPayload: (review: ReviewSubmission) => (inApp ? call<unknown>("review_payload", { review }) : Promise.resolve(review as unknown)),
  submitReview: (review: ReviewSubmission) => call<string>("submit_review", { review }),
  aiSetup: () => call<AiSetup>("ai_setup"),
  setAiKey: (provider: string, key: string) => call<AiSetup>("set_ai_key", { provider, key }),
  removeAiKey: (provider: string) => call<AiSetup>("remove_ai_key", { provider }),
  setAiModel: (model: string) => call<AiSetup>("set_ai_model", { model }),
  agentStates: () => call<AgentState[]>("agent_states"),
  connectAgent: (id: string) => call<AgentState[]>("connect_agent", { id }),
  disconnectAgent: (id: string) => call<AgentState[]>("disconnect_agent", { id }),
  aiEstimate: (review: Review) => (inApp ? call<{ chars: number; dollars: number }>("ai_estimate", { review }) : Promise.resolve({ chars: 40000, dollars: 0.28 })),
  // The same key as ai_store::key in Rust, so only a short string crosses over, however big the review.
  copyText: (text: string) => (inApp ? call<void>("copy_text", { text }) : navigator.clipboard.writeText(text)),
  savedAi: (review: Review) => (inApp ? call<SavedAi | null>("saved_ai", { key: reviewKey(review) }) : Promise.resolve(null)),
  // Work in progress, shared with the reviewer's own agent (Claude Code, Codex) through a file.
  loadDraft: (review: Review) => (inApp ? call<Draft | null>("load_draft", { key: reviewKey(review) }) : Promise.resolve(null)),
  patchDraft: (key: string, patch: DraftPatch) => (inApp ? call<Draft>("patch_draft", { key, patch }) : Promise.resolve(null)),
  setCurrentReview: (review: Review) => (inApp ? call<void>("set_current_review", { refs: review.prs.map((p) => `${p.summary.pr.owner}/${p.summary.pr.repo}#${p.summary.pr.number}`), key: reviewKey(review), title: review.briefing.title }) : Promise.resolve()),
  aiPrompt: (review: Review) => (inApp ? call<string>("ai_prompt", { review }) : Promise.resolve("(The exact prompt is shown here in the app.)")),
  runAi: (review: Review) => call<AiLayer>("run_ai", { review }),
  openUrl: (url: string) => (inApp ? call<void>("open_url", { url }) : Promise.resolve(void window.open(url, "_blank", "noopener"))),
};
