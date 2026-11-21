import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { api, reviewKey, type ReadyUpdate } from "./api";
import { Diff, Ext, FileHead, Folded, Ic, Logo } from "./ui";
import {
  LAYERS, hostName, plural, prKey, prLabel, prShort, samePr,
  type Account, type AgentState, type Draft, type DraftPatch, type Posted, type AiLayer, type AiProvider, type AiSetup, type CachedReview, type Answer, type GroupSuggestion, type Shop, type AuthStatus, type FileChange, type PrDetail, type PrSummary, type Review, type ReviewSubmission, type Saved, type Step, type Verdict,
} from "./types";

type Screen = "inbox" | "connections" | "plan" | "loading" | "briefing" | "map" | "step" | "routine" | "files" | "submit" | "done";

const EMPTY: Saved = { answers: {}, notes: {}, comments: {}, verdicts: {}, summary: "" };

// ---- Sharing work in progress with the reviewer's own agent (looksie_core::drafts). Both sides send only what changed.
type Plain = { answers: Record<string, string>; notes: Record<string, string>; comments: Record<string, string>; verdicts: Record<string, string>; summary: string };
const MAPS = ["answers", "notes", "comments", "verdicts"] as const;
const EMPTY_PLAIN: Plain = { answers: {}, notes: {}, comments: {}, verdicts: {}, summary: "" };
const tidy = (o: Record<string, string | undefined>) => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== "")) as Record<string, string>;
const plainOf = (s: Saved): Plain => ({ answers: tidy(s.answers), notes: tidy(s.notes), comments: tidy(s.comments), verdicts: tidy(s.verdicts), summary: s.summary });
const plainOfDraft = (d: Draft): Plain => ({ answers: d.answers ?? {}, notes: d.notes ?? {}, comments: d.comments ?? {}, verdicts: d.verdicts ?? {}, summary: d.summary ?? "" });
/** What changed from `a` to `b`; null when nothing did. */
function diffPlain(a: Plain, b: Plain): DraftPatch | null {
  const out: DraftPatch = {};
  let any = false;
  for (const m of MAPS) {
    const d: Record<string, string | null> = {};
    for (const [k, v] of Object.entries(b[m])) if (a[m][k] !== v) d[k] = v;
    for (const k of Object.keys(a[m])) if (!(k in b[m])) d[k] = null;
    if (Object.keys(d).length) { out[m] = d; any = true; }
  }
  if (a.summary !== b.summary) { out.summary = b.summary; any = true; }
  return any ? out : null;
}
function applyPlain(base: Plain, p: DraftPatch | null): Plain {
  if (!p) return base;
  const out: Plain = { ...base, answers: { ...base.answers }, notes: { ...base.notes }, comments: { ...base.comments }, verdicts: { ...base.verdicts } };
  for (const m of MAPS) for (const [k, v] of Object.entries((p[m] ?? {}) as Record<string, string | null>)) { if (v === null) delete out[m][k]; else out[m][k] = v; }
  if (p.summary !== undefined && p.summary !== null) out.summary = p.summary;
  return out;
}
/** Entries only `local` has: work never shared before (kept from before drafts were shared, say). */
function onlyIn(local: Plain, disk: Plain): DraftPatch | null {
  const out: DraftPatch = {};
  let any = false;
  for (const m of MAPS) {
    const d: Record<string, string> = {};
    for (const [k, v] of Object.entries(local[m])) if (!(k in disk[m])) d[k] = v;
    if (Object.keys(d).length) { out[m] = d; any = true; }
  }
  if (!disk.summary && local.summary) { out.summary = local.summary; any = true; }
  return any ? out : null;
}
const samePlain = (a: Plain, b: Plain) => diffPlain(a, b) === null;
const FLAG_TEXT = { look: "Look closely", problem: "Likely problem", none: "" } as const;
const ANSWER_TEXT: Record<Answer, string> = { fine: "Fine", change: "Needs changing", question: "Question" };
const VERDICT_TEXT: Record<Verdict, string> = { approve: "Approve", "request-changes": "Request changes", comment: "Comment" };
/** "5m", "3h", "2d" since an ISO date or a time in milliseconds. */
function ago(when: string | number): string {
  const ms = Date.now() - (typeof when === "number" ? when : Date.parse(when));
  if (!Number.isFinite(ms)) return "";
  const m = Math.max(0, Math.round(ms / 60000));
  if (m < 1) return "now";
  if (m < 60) return m + "m";
  const h = Math.round(m / 60);
  if (h < 24) return h + "h";
  const d = Math.round(h / 24);
  return d < 14 ? d + "d" : Math.round(d / 7) + "w";
}
const num = (n: number) => n.toLocaleString("en-US");
/** The cache key the backend uses for a set of pull requests. */
const cacheKey = (refs: string[]) => refs.map((r) => r.trim().toLowerCase()).sort().join("+");
/** Above this many connections the map only draws the selected card's, so a huge review stays readable and fast. */
const MAP_LINE_LIMIT = 150;
const ADDRESS = /[\w.-]+\/[\w.-]+(#|\/pull\/)\d+/;

const CLAIM_CLASS = { backed: "backed", contradicted: "contradicted", "not-found": "notfound", unchecked: "unchecked" } as const;

function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v ? (JSON.parse(v) as T) : fallback;
  } catch {
    return fallback;
  }
}
/** Saved answers and comments belong to the exact commits that were reviewed. New commits start a clean sheet. */
const savedKey = (r: Review) => "looksie:review:" + r.id + "@" + r.prs.map((p) => p.headSha.slice(0, 12)).join("+");
const aiKey = (r: Review) => "looksie:ai:" + r.id + "@" + r.prs.map((p) => p.headSha.slice(0, 12)).join("+");

function store(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage can be unavailable; the review still works for this session */
  }
}

export function App() {
  // The first time, follow the computer's own light or dark setting.
  const [dark, setDark] = useState(() => load("looksie:dark", typeof window !== "undefined" && !!window.matchMedia?.("(prefers-color-scheme: dark)").matches));
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  const [acctMenu, setAcctMenu] = useState(false);
  // The account menu opens towards whichever side has room (the chip moves left when the window is narrow).
  const [menuLeft, setMenuLeft] = useState(false);
  const [adding, setAdding] = useState<{ host: string } | null>(null);
  const [view, setView] = useState<"cards" | "compact">(() => load("looksie:view", "cards"));
  // Diffs one above the other, or old and new side by side. Remembered.
  const [split, setSplit] = useState(() => load("looksie:split-diffs", false));
  const [who, setWho] = useState<"all" | "review-requested" | "mine">("all");
  const [repoFilter, setRepoFilter] = useState("");
  const [query, setQuery] = useState("");
  const [sortBy, setSortBy] = useState<"new" | "big">("new");
  const [fresh, setFresh] = useState<PrSummary[] | null>(null);
  const [checking, setChecking] = useState(false);
  const [inboxAt, setInboxAt] = useState<number | null>(null);
  const [ready, setReady] = useState<Record<string, CachedReview>>({});
  const [closedRepos, setClosedRepos] = useState<string[]>([]);
  const [openGroups, setOpenGroups] = useState<string[]>([]);
  const [how, setHow] = useState(false);
  // The review (by id) whose over-limit steps the reviewer made into decisions. Another review starts with the limit on.
  const [allIn, setAllIn] = useState<string | null>(null);
  const [about, setAbout] = useState(false);
  // Group cards the reviewer has folded up. Remembered, so a folded group stays folded.
  const [folded, setFolded] = useState<string[]>(() => load<string[]>("looksie:folded-groups", []));
  // Diffs folded up on a decision, by step and file. Not remembered: a fresh look starts open.
  const [shut, setShut] = useState<string[]>([]);
  // The briefing's side panel scrolls on its own when it is taller than the window. Its edges fade where there is more.
  const sideRef = useRef<HTMLDivElement>(null);
  const [sideFade, setSideFade] = useState("");
  const [allNeeds, setAllNeeds] = useState(false);
  const sideEdges = () => {
    const el = sideRef.current;
    if (!el) return;
    const top = el.scrollTop > 4, more = el.scrollTop + el.clientHeight < el.scrollHeight - 4;
    setSideFade((top ? " ft" : "") + (more ? " fb" : ""));
  };
  useEffect(() => {
    const el = sideRef.current;
    if (!el) return;
    sideEdges();
    const ro = new ResizeObserver(sideEdges);
    ro.observe(el);
    [...el.children].forEach((c) => ro.observe(c));
    return () => ro.disconnect();
  });
  // Jumps to the next or previous file's diff on a decision, or back to the top.
  const jumpDiff = (dir: 1 | -1 | 0) => {
    const sc = document.querySelector<HTMLElement>(".stage .screen");
    if (!sc) return;
    if (dir === 0) { sc.scrollTo({ top: 0, behavior: "smooth" }); return; }
    const line = sc.getBoundingClientRect().top + 64; // just under the bar that stays in view
    const cards = [...sc.querySelectorAll<HTMLElement>("[data-diff]")];
    const gap = (c: HTMLElement) => c.getBoundingClientRect().top - line;
    const target = dir > 0 ? cards.find((c) => gap(c) > 4) : [...cards].reverse().find((c) => gap(c) < -4);
    if (target) sc.scrollBy({ top: gap(target), behavior: "smooth" });
    else if (dir < 0) sc.scrollTo({ top: 0, behavior: "smooth" });
  };
  useEffect(() => store("looksie:folded-groups", folded), [folded]);
  const [update, setUpdate] = useState<ReadyUpdate | null>(null);
  const [updateNote, setUpdateNote] = useState<"hidden" | "notes" | "installing" | null>(null);
  const [checkState, setCheckState] = useState<string | null>(null);
  const [version, setVersion] = useState("");
  const [aiAuto, setAiAuto] = useState(() => load("looksie:ai-auto", false));
  const [aiRun, setAiRun] = useState<{ started: number; expect: number; decisions: number } | null>(null);
  const [, setNow] = useState(0);
  const [ai, setAi] = useState<AiSetup | null>(null);
  const [agents, setAgents] = useState<AgentState[] | null>(null);
  // Waiting for the reviewer's own agent (Claude Code, Codex) to save an answer: when they started, so it can stop looking.
  const [agentWait, setAgentWait] = useState<number | null>(null);
  const [aiCost, setAiCost] = useState<number | null>(null);
  const [inbox, setInbox] = useState<PrSummary[] | null>(null);
  const [inboxError, setInboxError] = useState<string | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const [address, setAddress] = useState("");
  const [screen, setScreen] = useState<Screen>("inbox");
  const [review, setReview] = useState<Review | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loadingWhat, setLoadingWhat] = useState<string[]>([]);
  const [saved, setSaved] = useState<Saved>(EMPTY);
  const [stepIdx, setStepIdx] = useState(0);
  const [dir, setDir] = useState(1);
  const [tick, setTick] = useState(0);
  const [outline, setOutline] = useState(false);
  const [flagFirst, setFlagFirst] = useState(false);
  const [fileSel, setFileSel] = useState<{ pr: string; path: string } | null>(null);
  const [fileFind, setFileFind] = useState("");
  const [fileShow, setFileShow] = useState(300);
  const [payloads, setPayloads] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState<{ pr: string; verdict: Verdict; url?: string; error?: string }[] | null>(null);
  const [aiOn, setAiOn] = useState(() => load("looksie:ai-on", true));
  const [aiLayer, setAiLayer] = useState<AiLayer | null>(null);
  const [aiBusy, setAiBusy] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [promptText, setPromptText] = useState<string | null>(null);
  const [shop, setShop] = useState<Shop | null>(null);
  const [offer, setOffer] = useState<{ left: number; max: number } | null>(null);
  const [pending, setPending] = useState<{ refs: string[]; name?: string } | null>(null);
  const [suggestions, setSuggestions] = useState<GroupSuggestion[]>([]);
  const [dismissed, setDismissed] = useState<string[]>(() => load<string[]>("looksie:dismissed-groups", []));

  useEffect(() => {
    store("looksie:dark", dark);
    // The window's title bar (macOS and Windows) follows too, so the whole window matches.
    api.setWindowTheme(dark).catch(() => undefined);
  }, [dark]);
  useEffect(() => store("looksie:view", view), [view]);
  useEffect(() => store("looksie:split-diffs", split), [split]);
  useEffect(() => store("looksie:ai-auto", aiAuto), [aiAuto]);
  // While the AI works, redraw once a second so the progress and time left move.
  useEffect(() => {
    if (!aiBusy) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [aiBusy]);
  useEffect(() => store("looksie:ai-on", aiOn), [aiOn]);
  // Answers, notes and comments are also kept on disk, shared with the reviewer's own Claude Code or Codex
  // (looksie --mcp), so a review can be started in one and finished in the other. Each side sends only what changed,
  // applied on top of what is on disk, so neither can wipe what the other just saved.
  const synced = useRef<{ key: string; base: Plain; at: number } | null>(null);
  const [posted, setPosted] = useState<Record<string, Posted>>({});
  const draftTimer = useRef<number | undefined>(undefined);
  // The review being opened, set at once (reviewRef only catches up after a render).
  const openKey = useRef<string | null>(null);
  // A save waiting for its 400ms: sent at once if another review is opened meanwhile.
  const waitingSave = useRef<{ key: string; patch: DraftPatch; now: Plain } | null>(null);
  const sendPatch = (key: string, patch: DraftPatch, now: Plain) => {
    waitingSave.current = null;
    api.patchDraft(key, patch).then((d) => {
      if (!d) return;
      store("looksie:shared:" + key, true);
      const sy = synced.current;
      // A newer copy (the agent's) may already have been taken while this was on its way.
      if (sy && sy.key === key && d.updatedAt > sy.at) adopt(key, d, now);
    }).catch(() => undefined);
  };
  /** Take the copy on disk, keeping any edit made here since `since`. */
  const adopt = (key: string, d: Draft, since: Plain) => {
    synced.current = { key, base: plainOfDraft(d), at: d.updatedAt };
    setPosted(d.posted ?? {});
    setSaved((cur) => {
      const next = applyPlain(plainOfDraft(d), diffPlain(since, plainOf(cur)));
      return samePlain(next, plainOf(cur)) ? cur : (next as Saved);
    });
  };
  useEffect(() => {
    if (!review) return;
    store(savedKey(review), saved);
    const key = reviewKey(review);
    const sy = synced.current;
    if (!sy || sy.key !== key) return;
    window.clearTimeout(draftTimer.current);
    const now = plainOf(saved);
    const patch = diffPlain(sy.base, now);
    if (!patch) return;
    waitingSave.current = { key, patch, now };
    draftTimer.current = window.setTimeout(() => sendPatch(key, patch, now), 400);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [saved, review]);
  const takeDraft = (r: Review) => {
    const key = reviewKey(r);
    return api.loadDraft(r).then((d) => {
      const sy = synced.current;
      if (!d || !sy || sy.key !== key || d.updatedAt <= sy.at) return;
      adopt(key, d, sy.base);
    }).catch(() => undefined);
  };
  /** On opening a review: the disk copy wins, and anything only kept here is added to it. */
  const startSync = (r: Review, local: Saved) => {
    const key = reviewKey(r);
    // A save still waiting for the review being left goes now, rather than being lost.
    const wait = waitingSave.current;
    if (wait && wait.key !== key) { window.clearTimeout(draftTimer.current); sendPatch(wait.key, wait.patch, wait.now); }
    openKey.current = key;
    synced.current = null;
    setPosted({});
    api.loadDraft(r).then((d) => {
      if (openKey.current !== key) return;
      if (!d || !d.updatedAt) {
        synced.current = { key, base: EMPTY_PLAIN, at: 0 };
        setSaved((cur) => ({ ...cur })); // shares what is kept here, if anything
        return;
      }
      synced.current = { key, base: plainOfDraft(d), at: d.updatedAt };
      setPosted(d.posted ?? {});
      // Answers only kept here are added the first time this review is shared; after that the shared copy is the
      // truth, so something removed in the agent stays removed.
      const first = !load<boolean>("looksie:shared:" + key, false);
      store("looksie:shared:" + key, true);
      setSaved(applyPlain(plainOfDraft(d), first ? onlyIn(plainOf(local), plainOfDraft(d)) : null) as Saved);
    }).catch(() => { if (openKey.current === key) synced.current = { key, base: plainOf(local), at: 0 }; });
  };
  useEffect(() => {
    if (!review) return;
    const r = review;
    const t = window.setInterval(() => { if (document.visibilityState === "visible") takeDraft(r); }, 3000);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [review]);

  const active = accounts?.find((a) => a.active) ?? null;
  const auth: AuthStatus | null = active ? { login: active.login, source: active.sourceText } : null;
  const inboxKey = (id: string) => "looksie:inbox:" + id;
  const shownRef = useRef<PrSummary[] | null>(null);
  useEffect(() => { shownRef.current = inbox; }, [inbox]);
  const showList = (list: PrSummary[]) => {
    setInbox(list);
    api.suggestGroups(list).then(setSuggestions).catch(() => setSuggestions([]));
  };
  /** Shows what was cached for this account straight away, then checks GitHub in the background. */
  const showCached = (id: string) => {
    const c = load<{ at: number; list: PrSummary[] } | null>(inboxKey(id), null);
    if (c) { showList(c.list); setInboxAt(c.at); }
  };
  /** Fetches the inbox. A list already on screen is never reshuffled under the pointer: rows update in place,
   *  and new or closed pull requests wait behind a button until the reviewer asks for them. */
  const refresh = (acct: Account | null = active, quietly = false) => {
    if (!acct) return;
    setInboxError(null);
    setChecking(true);
    api.cachedReviews().then(setReady).catch(() => undefined);
    api.listInbox().then((list) => {
      store(inboxKey(acct.id), { at: Date.now(), list });
      setInboxAt(Date.now());
      const shown = shownRef.current;
      if (!shown || !quietly) { showList(list); setFresh(null); return; }
      const byKey = new Map(list.map((p) => [prKey(p.pr), p]));
      const added = list.filter((p) => !shown.some((x) => samePr(x.pr, p.pr)));
      const gone = shown.filter((x) => !byKey.has(prKey(x.pr)));
      showList(shown.map((x) => byKey.get(prKey(x.pr)) ?? x));
      setFresh(added.length || gone.length ? list : null);
    }).catch((e) => setInboxError(String(e.message ?? e))).finally(() => setChecking(false));
  };
  const lookAgain = () => api.listAccounts().then((list) => {
    setAccounts(list);
    const a = list.find((x) => x.active);
    if (a) { store("looksie:account", a.id); if (!shownRef.current) showCached(a.id); refresh(a, true); }
  }).catch(() => setAccounts([]));
  const switchTo = (p: Promise<Account[]>) => p.then((list) => {
    setAccounts(list);
    setAcctMenu(false);
    const a = list.find((x) => x.active);
    if (a && a.id !== active?.id) {
      store("looksie:account", a.id);
      setInbox(null); shownRef.current = null; setFresh(null); setPicked([]); setRepoFilter("");
      showCached(a.id);
      refresh(a, true);
    }
    return list;
  });
  /** Checks for a newer Looksie. Quietly, unless the reviewer asked (then it says what happened). */
  const checkForUpdate = (asked = false) => {
    store("looksie:update-checked", Date.now());
    if (asked) setCheckState("Checking…");
    api.findUpdate().then((u) => {
      if (u) { setUpdate(u); setUpdateNote(null); }
      if (asked) setCheckState(u ? `Looksie ${u.version} is ready to install.` : "You have the latest version.");
    }).catch((e) => {
      // Say why, so a problem can be reported; and offer the website as a way round it.
      if (asked) setCheckState(`Could not check: ${String((e as Error)?.message ?? e).slice(0, 160)}`);
    });
  };
  useEffect(() => {
    api.version().then(setVersion).catch(() => undefined);
    // Once shortly after starting, then whenever a day has passed while it is open.
    const first = window.setTimeout(() => checkForUpdate(), 5000);
    // Left open, it looks again every 6 hours.
    const daily = window.setInterval(() => { if (Date.now() - load<number>("looksie:update-checked", 0) > 6 * 3_600_000) checkForUpdate(); }, 3_600_000);
    return () => { window.clearTimeout(first); window.clearInterval(daily); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    const last = load<string | null>("looksie:account", null);
    if (last) showCached(last);
    lookAgain();
    api.aiSetup().then(setAi).catch(() => setAi(null));
    api.agentStates().then(setAgents).catch(() => setAgents(null));
    // Asks the store about a key once a week; otherwise just reads the plan.
    api.recheckLicence().then(setShop).catch(() => api.shop().then(setShop).catch(() => setShop(null)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const go = (s: Screen) => {
    setScreen(s); setTick((t) => t + 1); setOutline(false);
    // The trial counts down while the app is open, so the plan is read afresh each time it is shown.
    if (s === "plan") {
      api.shop().then(setShop).catch(() => undefined);
      // The launch offer is limited: show it only while places are left, with how many once some have gone.
      api.offerLeft().then(setOffer).catch(() => setOffer(null));
    }
  };
  // Moving between steps slides the step in from the side it comes from; it does not replay the page's entrance.
  const goStep = (i: number) => { setDir(i >= stepIdx ? 1 : -1); setStepIdx(i); if (screen !== "step") go("step"); else setOutline(false); };
  // A new page, or a new step, starts at the top. Picking a card on the map does not.
  const lastScreen = useRef(screen);
  useEffect(() => {
    if (lastScreen.current !== screen || screen === "step") document.querySelector(".stage .screen")?.scrollTo({ top: 0 });
    lastScreen.current = screen;
  }, [screen, stepIdx]);
  const openFile = (f: { pr: PrDetail["summary"]["pr"]; path: string }) => { setFileSel({ pr: prKey(f.pr), path: f.path }); go("files"); };

  const plan = shop?.status.plan ?? "pro"; // if the plan cannot be read, do not get in the way
  // The trial counts down while the app is open: read the plan again every hour, so the tab and the note stay true.
  useEffect(() => {
    const t = window.setInterval(() => api.recheckLicence().then(setShop).catch(() => undefined), 3_600_000);
    return () => window.clearInterval(t);
  }, []);
  // A note in the inbox for the last 3 days of the trial, and once when it has finished. Hidden notes stay hidden,
  // but each new day of the countdown shows again.
  const [trialSeen, setTrialSeen] = useState<string>(() => load<string>("looksie:trial-note", ""));
  useEffect(() => store("looksie:trial-note", trialSeen), [trialSeen]);
  const trialNote = (() => {
    const left = shop?.status.trialDaysLeft ?? null;
    if (plan === "trial" && left !== null && left <= 3) {
      const id = `ending-${left}`;
      return trialSeen === id ? null : { id, text: left <= 1 ? "Your free trial ends today. Public repositories stay free; private ones will need Pro." : `Your free trial ends in ${left} days. Public repositories stay free; private ones will need Pro.` };
    }
    if (plan === "free" && shop?.status.trialUsed) {
      return trialSeen === "finished" ? null : { id: "finished", text: "Your free trial has finished. Public repositories stay free; Pro opens private ones again." };
    }
    return null;
  })();
  const [moved, setMoved] = useState(false);
  /** Fetches and shows a review. `open` is the front door; this is the part after the Pro check. */
  /** The latest commits for these pull requests, when the inbox knows them all. */
  const headsOf = (refs: string[]) => {
    const heads = refs.map((r) => (inbox ?? []).find((p) => prKey(p.pr).toLowerCase() === r.trim().toLowerCase())?.headSha ?? "");
    return heads.every(Boolean) ? heads : undefined;
  };
  /** "ready": prepared earlier for these exact commits, opens instantly. "stale": prepared, but there are new commits. */
  const readiness = (refs: string[]) => {
    const e = ready[cacheKey(refs)];
    if (!e) return null;
    const now = headsOf(refs);
    if (!now) return null;
    return [...e.heads].sort().join() === [...now].sort().join() ? "ready" : "stale";
  };
  const loadReview = (refs: string[], name?: string, again = false) => {
    setLoadError(null);
    setLoadingWhat(refs);
    setReview(null);
    setSent(null);
    setPayloads(null);
    go("loading");
    api.buildReview(refs, name, headsOf(refs), again).then((r) => {
      api.cachedReviews().then(setReady).catch(() => undefined);
      // If this pull request was reviewed before at an older commit, say so rather than silently reusing or dropping answers.
      const last = load<string | null>("looksie:last:" + r.id, null);
      setMoved(last !== null && last !== savedKey(r));
      store("looksie:last:" + r.id, savedKey(r));
      setReview(r);
      const local = { ...EMPTY, ...load<Partial<Saved>>(savedKey(r), {}) };
      setSaved(local);
      api.setCurrentReview(r).catch(() => undefined);
      startSync(r, local);
      // An earlier answer: from the app's AI folder (written here or by the reviewer's own agent), else from before 0.2.
      const keptAi = load<AiLayer | null>(aiKey(r), null);
      setAiLayer(keptAi);
      setAgentWait(null);
      setAiCost(null);
      api.savedAi(r).then((s) => {
        if (s && reviewRef.current && aiKey(reviewRef.current) === aiKey(r)) setAiLayer(s.layer);
        // Only if the reviewer has asked for it (Settings, Your AI): the summary is then written by itself the first time.
        else if (!s && !keptAi && aiOn && aiAuto && ai?.hasKey) askAi(r);
      }).catch(() => undefined);
      setAiError(null);
      setStepIdx(0);
      setFileSel(null);
      setPicked([]);
      go("briefing");
    }).catch((e) => {
      const msg = String(e.message ?? e);
      if (msg.startsWith("pro-required")) {
        setPending({ refs, name });
        go("plan");
      } else {
        setLoadError(msg);
      }
    });
  };
  const open = (refs: string[], name?: string) => {
    // Private repositories are part of Pro. Anyone can pick one; without Pro they are offered the trial first.
    const knownPrivate = (inbox ?? []).some((p) => p.private && refs.includes(prKey(p.pr)));
    if (plan === "free" && knownPrivate) {
      setPending({ refs, name });
      go("plan");
      return;
    }
    loadReview(refs, name);
  };
  const gotPro = (next: Shop) => {
    setShop(next);
    if (pending && next.status.plan !== "free") {
      const p = pending;
      setPending(null);
      loadReview(p.refs, p.name);
    }
  };
  const groupKey = (g: GroupSuggestion) => g.prs.map(prKey).sort().join("+");

  // With the limit lifted, steps over it become decisions, after the others and strongest first (the engine's order).
  const lifted = !!review && allIn === review.id;
  const steps: Step[] = (() => {
    const s = review?.steps ?? [];
    if (!lifted) return s;
    const as = s.map((x) => (x.overLimit ? { ...x, tier: "decide" as const } : x));
    return [...as.filter((x) => x.tier === "decide"), ...as.filter((x) => x.tier !== "decide")];
  })();
  const overLimit = (review?.steps ?? []).filter((s) => s.overLimit).length;
  const cur: Step | undefined = steps[Math.min(stepIdx, Math.max(steps.length - 1, 0))];
  // The AI layer sits on top of the facts. Switch it off and everything below falls back to rules only.
  // AI comes from the reviewer's own Anthropic or OpenAI API key, or from their own Claude Code or Codex (Settings, Your AI).
  const canAsk = !!ai?.hasKey;
  const modelName = ai?.models.find((m) => m.id === ai.model)?.name ?? "the AI";
  const aiCompany = ai?.providers.find((p) => p.id === ai.provider);
  // The rough cost, worked out again when the review, the model or the key changes: prices differ a lot between models.
  useEffect(() => {
    setAiCost(null);
    if (!review || !ai?.hasKey) return;
    let live = true;
    api.aiEstimate(review).then((e) => { if (live) setAiCost(e.dollars); }).catch(() => undefined);
    return () => { live = false; };
  }, [review, ai?.model, ai?.hasKey]);
  const layer = aiOn ? aiLayer : null;
  const aiStep = (s: Step) => layer?.steps.find((x) => x.id === s.id);
  const RANK = { none: 0, look: 1, problem: 2 } as const;
  const flagOf = (s: Step) => { const a = aiStep(s)?.flag ?? "none"; return RANK[a] > RANK[s.flag] ? a : s.flag; };
  const stateOf = (s: Step) => saved.answers[s.id] ?? (flagOf(s) !== "none" ? flagOf(s) : "todo");
  const markOf = (s: Step) => (saved.answers[s.id] ? ANSWER_TEXT[saved.answers[s.id]!] : flagOf(s) !== "none" ? FLAG_TEXT[flagOf(s)] : "To do");
  const reviewRef = useRef<Review | null>(null);
  useEffect(() => { reviewRef.current = review; }, [review]);
  const askAi = async (target?: unknown) => {
    const r = (target as Review | undefined)?.prs ? (target as Review) : review;
    if (!r || !canAsk) return;
    setAiBusy(true);
    const started = Date.now();
    // How long it should take: a model from real runs (start-up, then time per thousand characters of prompt),
    // corrected by how this computer's AI has done lately (the middle of its last five runs).
    const chars = await api.aiPrompt(r).then((p) => p.length).catch(() => 20000);
    const model = 10000 + (chars / 1000) * 1500;
    const past = load<number[]>("looksie:ai-factors-api", []);
    const factor = past.length ? [...past].sort((a, b) => a - b)[Math.floor(past.length / 2)] : 1;
    setAiRun({ started, expect: model * factor, decisions: r.steps.filter((x) => x.tier === "decide").length });
    setAiError(null);
    try {
      const l = await api.runAi(r);
      const took = Date.now() - started;
      store("looksie:ai-factors-api", [...past, Math.min(4, Math.max(0.25, took / model))].slice(-5));
      // The reviewer may have moved on to another review while the AI was working.
      if (reviewRef.current && aiKey(reviewRef.current) === aiKey(r)) setAiLayer(l);
      setAiOn(true);
    } catch (e) {
      setAiError(String((e as Error).message ?? e));
    } finally {
      setAiBusy(false);
      setAiRun(null);
    }
  };
  // While the reviewer's own agent works on this review, look every few seconds for its saved answer (for up to 20 minutes).
  // Coming back to the window looks once too, for answers saved without pressing Copy here first.
  useEffect(() => {
    if (!review || aiLayer) return;
    const look = () => api.savedAi(review).then((s) => {
      if (s && reviewRef.current && aiKey(reviewRef.current) === aiKey(review)) { setAiLayer(s.layer); setAiOn(true); setAgentWait(null); }
    }).catch(() => undefined);
    window.addEventListener("focus", look);
    const t = agentWait ? window.setInterval(() => { if (Date.now() - agentWait > 20 * 60_000) setAgentWait(null); else look(); }, 4000) : undefined;
    return () => { window.removeEventListener("focus", look); if (t) window.clearInterval(t); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [review, aiLayer, agentWait]);
  /** Where the AI has got to, in words, and roughly how long is left. The API does not report progress, so this is an estimate. */
  const aiProgress = () => {
    if (!aiRun) return { stage: `Starting ${modelName}`, left: "", pct: 3 };
    const gone = Date.now() - aiRun.started;
    const f = gone / aiRun.expect;
    const stage = f < 0.12 ? `Sending the changes to ${modelName}` : f < 0.9 ? `${modelName} is reading ${plural(aiRun.decisions, "decision")}` : "Checking its answers against the code";
    // A range, not a falsely exact countdown: answers vary a lot from run to run.
    const lo = Math.max(0, (aiRun.expect * 0.75 - gone) / 1000), hi = Math.max(0, (aiRun.expect * 1.3 - gone) / 1000);
    const span = (a: number, b: number) => b >= 90 ? `${Math.max(1, Math.round(a / 60))}${Math.round(b / 60) > Math.max(1, Math.round(a / 60)) ? "–" + Math.round(b / 60) : ""} minute${Math.round(b / 60) > 1 ? "s" : ""}` : `${Math.max(5, Math.round(a / 5) * 5)}–${Math.max(10, Math.round(b / 5) * 5)} seconds`;
    const left = hi < 3 ? "Nearly done; taking a little longer than usual" : `About ${span(lo, hi)} left`;
    return { stage, left, pct: Math.min(95, Math.max(3, f * 100)) };
  };
  /** The pull requests a step touches, each opening on its host (GitHub today). */
  const prLinks = (st: Step) => {
    const seen: PrDetail["summary"]["pr"][] = [];
    for (const f of st.files) if (!seen.some((x) => samePr(x, f.pr))) seen.push(f.pr);
    return seen.map((pr) => {
      const d = review?.prs.find((p) => samePr(p.summary.pr, pr));
      return d?.summary.url
        ? <button key={prKey(pr)} className="tag ext" title={`Open ${prLabel(pr)} on ${hostName(pr.host)}`} onClick={() => api.openUrl(d.summary.url)}>{prShort(pr)}<Ic name="external" /></button>
        : <span key={prKey(pr)} className="mono">{prShort(pr)}</span>;
    });
  };
  const noRule = (st: Step) => {
    const d = review?.prs.find((p) => st.files[0] && samePr(p.summary.pr, st.files[0].pr));
    return !d || !d.body.trim()
      ? "There is no description to check this against. Read the change: is it what this pull request is for, and is it safe to merge?"
      : "Nothing here tripped a rule. Read the change: does it do what the description says, and nothing more?";
  };
  const fileOf = (pr: PrDetail["summary"]["pr"], path: string): FileChange | undefined => review?.files.find((f) => samePr(f.pr, pr) && f.path === path);
  const stepOfFile = (f: FileChange) => steps.findIndex((s) => s.files.some((x) => samePr(x.pr, f.pr) && x.path === f.path));
  // The engine puts decisions first, then routine changes. Only decisions count towards "answered".
  const decisions = steps.filter((s) => s.tier === "decide");
  const routine = steps.filter((s) => s.tier === "glance");
  const answered = decisions.filter((s) => saved.answers[s.id]).length;
  const total = decisions.length;
  const curIsRoutine = cur?.tier === "glance";
  const setComment = (key: string, text: string) => setSaved((v) => { const c = { ...v.comments }; if (text) c[key] = text; else delete c[key]; return { ...v, comments: c }; });

  // ---------- what would be sent ----------
  const submissions: ReviewSubmission[] = useMemo(() => {
    if (!review) return [];
    return review.prs.map((p) => {
      const pr = p.summary.pr;
      const comments: ReviewSubmission["comments"] = [];
      const mine = review.steps.filter((s) => s.files.length && samePr(s.files[0].pr, pr));
      for (const s of mine) {
        const a = saved.answers[s.id];
        // A note is always sent, whatever the answer: the box promises it becomes a review comment.
        const note = (saved.notes[s.id] ?? "").trim();
        if (!note && a !== "change" && a !== "question") continue;
        const body = note || (a === "change" ? `This needs changing: ${s.title}` : `Question about: ${s.title}`);
        const f = review.files.find((x) => samePr(x.pr, pr) && x.path === s.files[0].path);
        const line = f?.hunks.flatMap((h) => h.lines).find((l) => l.kind === "add" && l.newNo !== null)?.newNo;
        comments.push({ anchor: f && line ? { path: f.path, side: "new", startLine: line, endLine: line } : null, body });
      }
      for (const [key, body] of Object.entries(saved.comments)) {
        const [k, path, side, no] = key.split("|");
        if (k === prKey(pr) && body) comments.push({ anchor: { path, side: side as "old" | "new", startLine: Number(no), endLine: Number(no) }, body });
      }
      const touching = review.steps.filter((s) => s.files.some((f) => samePr(f.pr, pr)));
      const suggested: Verdict = touching.some((s) => saved.answers[s.id] === "change") ? "request-changes" : touching.length > 0 && touching.every((s) => saved.answers[s.id] === "fine") ? "approve" : "comment";
      const own = !!auth && p.summary.author.toLowerCase() === auth.login.toLowerCase();
      const verdict = own ? "comment" : (saved.verdicts[prKey(pr)] ?? suggested);
      return { pr, commit: p.headSha, verdict, summary: saved.summary.trim(), comments };
    });
  }, [review, saved, auth]);

  // A review with no verdict to give, no summary and no comments has nothing in it, and the host would refuse it.
  // A pull request already reviewed at this commit (here or from the reviewer's agent) is never posted again.
  const postedHere = (s: ReviewSubmission) => posted[prKey(s.pr)]?.commit === s.commit;
  const sendable = submissions.filter((s) => !postedHere(s) && (s.verdict === "approve" || s.summary !== "" || s.comments.length > 0));
  const submit = async () => {
    if (!review || sendable.length === 0) return;
    // The agent may have posted a moment ago: check the shared copy once more.
    const fresh = await api.loadDraft(review).catch(() => null);
    const already = fresh?.posted ?? {};
    const toSend = sendable.filter((s) => already[prKey(s.pr)]?.commit !== s.commit);
    if (fresh?.posted) setPosted(fresh.posted);
    if (toSend.length === 0) return;
    const n = toSend.length;
    if (!window.confirm(`Post ${plural(n, "review")} to ${hostName(review.prs[0].summary.pr.host)} as ${auth?.login ?? "you"}? This cannot be undone from Looksie.`)) return;
    setSending(true);
    const out: NonNullable<typeof sent> = [];
    for (const s of toSend) {
      try {
        const url = await api.submitReview(s);
        out.push({ pr: prLabel(s.pr), verdict: s.verdict, url });
        api.patchDraft(reviewKey(review), { posted: { [prKey(s.pr)]: { url, commit: s.commit, at: Date.now() } } }).then((d) => d && setPosted(d.posted ?? {})).catch(() => undefined);
      } catch (e) {
        out.push({ pr: prLabel(s.pr), verdict: s.verdict, error: String((e as Error).message ?? e) });
      }
    }
    setSending(false);
    setSent(out);
    go("done");
  };

  const inReview = !!review && screen !== "inbox" && screen !== "connections" && screen !== "loading" && screen !== "plan";
  const anim = tick % 2 ? "a" : "b";
  const screenCls = "screen fade-" + anim;
  // The map's connection lines: drawn over the grid, from the step that is built on to the step that uses it.
  const matrixRef = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState<{ d: string; hot: boolean }[]>([]);
  const [hidden, setHidden] = useState(0);
  useLayoutEffect(() => {
    const box = matrixRef.current;
    if (screen !== "map" || !box) { setEdges([]); return; }
    const draw = () => {
      const base = box.getBoundingClientRect();
      // Each card, and the cell it sits in (lines run in the gaps between cells, never over cards).
      const at = (id: string) => {
        const el = box.querySelector(`[data-id="${CSS.escape(id)}"]`);
        if (!el) return null;
        const r = el.getBoundingClientRect();
        const c = (el.closest(".cell") ?? el).getBoundingClientRect();
        return { l: r.left - base.left, r: r.right - base.left, t: r.top - base.top, b: r.bottom - base.top, cx: (r.left + r.right) / 2 - base.left, cy: (r.top + r.bottom) / 2 - base.top, cl: c.left - base.left, cr: c.right - base.left, cell: el.closest(".cell") };
      };
      const gap = parseFloat(getComputedStyle(box).columnGap) || 40;
      const curId = steps[stepIdx]?.id;
      const all: { on: string; to: string; hot: boolean }[] = [];
      for (const st of steps) for (const on of st.buildsOn) all.push({ on, to: st.id, hot: st.id === curId || on === curId });
      // A huge review can have thousands of connections. Past a limit, only the selected card's are drawn.
      const many = all.length > MAP_LINE_LIMIT;
      setHidden(many ? all.filter((x) => !x.hot).length : 0);
      const out: { d: string; hot: boolean }[] = [];
      const R = 8;
      // Right-angled lines with rounded corners: out of one card, along the gap, into the other.
      const route = (x1: number, y1: number, xg: number, y2: number, x2: number) => {
        if (Math.abs(y2 - y1) < 2) return `M${x1},${y1} L${x2},${y2}`;
        const dy = y2 > y1 ? 1 : -1, d1 = xg > x1 ? 1 : -1, d2 = x2 > xg ? 1 : -1;
        const r = Math.min(R, Math.abs(y2 - y1) / 2);
        return `M${x1},${y1} L${xg - d1 * r},${y1} Q${xg},${y1} ${xg},${y1 + dy * r} L${xg},${y2 - dy * r} Q${xg},${y2} ${xg + d2 * r},${y2} L${x2},${y2}`;
      };
      for (const e of all) {
        if (many && !e.hot) continue;
        const a = at(e.on), b = at(e.to);
        if (!a || !b) continue;
        let d: string;
        if (b.cl > a.cr - 1) d = route(a.r, a.cy, b.cl - gap / 2, b.cy, b.l);          // to a column on the right
        else if (b.cr < a.cl + 1) d = route(a.l, a.cy, b.cr + gap / 2, b.cy, b.r);     // to a column on the left
        else if (a.cell && a.cell === b.cell) d = b.t > a.b ? `M${a.cx},${a.b} L${b.cx},${b.t}` : `M${a.cx},${a.t} L${b.cx},${b.b}`; // stacked in one box: a short straight arrow
        else d = route(a.r, a.cy, a.cr + gap / 2, b.cy, b.r);                          // same column, another row: round by the gap on its right
        out.push({ d, hot: e.hot });
      }
      out.sort((x, y) => Number(x.hot) - Number(y.hot));
      setEdges(out);
    };
    draw();
    const ro = new ResizeObserver(draw);
    ro.observe(box);
    return () => ro.disconnect();
  }, [screen, review, stepIdx, saved]);
  const diffSwitch = (
    <div className="segc sm" role="group" aria-label="How to show changes">
      <button className={split ? "" : "on"} aria-pressed={!split} title="Old and new lines in one column" onClick={() => setSplit(false)}>Unified</button>
      <button className={split ? "on" : ""} aria-pressed={split} title="The old file on the left, the new one on the right" onClick={() => setSplit(true)}>Side by side</button>
    </div>
  );
  // Decisions can be walked through one at a time, or seen all at once on the map.
  const viewSwitch = (
    <div className="segc" role="group" aria-label="How to see the decisions">
      <button className={screen === "step" || screen === "routine" ? "on" : ""} aria-pressed={screen === "step" || screen === "routine"} onClick={() => goStep(stepIdx)}><Ic name="cards" />One at a time</button>
      <button className={screen === "map" ? "on" : ""} aria-pressed={screen === "map"} onClick={() => go("map")}><Ic name="grid" />Map</button>
      <button className={screen === "files" ? "on" : ""} aria-pressed={screen === "files"} onClick={() => go("files")}><Ic name="rows" />Files</button>
    </div>
  );
  const tab = (s: Screen, label: string) => (
    <button className={"rtab" + (screen === s || (s === "submit" && screen === "done") ? " on" : "")} onClick={() => go(s)}>{label}</button>
  );

  return (
    <div className={"app " + (dark ? "t-dark" : "t-light")}>
      <nav className="rail" aria-label="Looksie">
        <button className="logobtn" aria-label="About Looksie" title="About Looksie" onClick={() => setAbout(true)}><Logo /></button>
        <button className={"tab" + (!inReview && screen !== "connections" && screen !== "plan" ? " on" : "")} onClick={() => go("inbox")}><Ic name="inbox" />Inbox</button>
        <button className={"tab" + (screen === "connections" ? " on" : "")} onClick={() => go("connections")}><Ic name="gear" />Settings</button>
        <div className="sp" />
        {shop && <button className={"tab" + (screen === "plan" ? " on" : "")} onClick={() => go("plan")}><Ic name="badge" />{plan === "pro" ? "Pro" : plan === "trial" ? `Trial · ${shop.status.trialDaysLeft}d` : "Free"}</button>}
        {canAsk || aiLayer || !ai ? (
          <button className={"switch" + (aiOn ? " on" : "")} aria-pressed={aiOn} title="Switch the AI layer on or off. Nothing is re-run: it only shows or hides what the AI added." onClick={() => setAiOn(!aiOn)}><Ic name="spark" />AI {aiOn ? "on" : "off"}</button>
        ) : (
          <button className={"switch" + (screen === "connections" ? " on" : "")} title="Add AI with your own API key, or your own Claude Code or Codex. Everything works without AI." onClick={() => go("connections")}><Ic name="spark" />Set up AI</button>
        )}
        <button className="switch" aria-label={dark ? "Switch to light mode" : "Switch to dark mode"} onClick={() => setDark(!dark)}><Ic name="moon" />Theme</button>
      </nav>

      <div className="stage">
        {inReview && review && (
          <header className="rhead">
            <button className="backb" aria-label="Back to the inbox" onClick={() => go("inbox")}><Ic name="back" /></button>
            <span className="rname">{review.briefing.title}</span>
            <span className="scope">{review.prs.length > 1 ? plural(review.prs.length, "pull request") : prLabel(review.prs[0].summary.pr)}</span>
            {aiBusy && <button className="tag ai" title="Your AI is working on this review" onClick={() => go("briefing")}><span className="spin sm" />AI working{aiRun ? ` · ${Math.max(0, Math.round((aiRun.expect - (Date.now() - aiRun.started)) / 1000))}s` : ""}</button>}
            <button className="tag" title="Fetch the pull requests again and prepare this review from scratch" onClick={() => loadReview(review.prs.map((p) => prKey(p.summary.pr)), review.prs.length > 1 ? review.briefing.title : undefined, true)}><Ic name="refresh" />Refresh</button>
            {review.prs.some((p) => p.summary.private) && plan === "trial" && <button className="tag" onClick={() => go("plan")}>Trial · {plural(shop?.status.trialDaysLeft ?? 0, "day")} left</button>}
            <nav className="rtabs" aria-label="This review">
              {tab("briefing", "Briefing")}
              <button className={"rtab" + (screen === "step" || screen === "routine" || screen === "map" || screen === "files" ? " on" : "")} onClick={() => goStep(curIsRoutine ? 0 : stepIdx)}>Decisions · {total}</button>
              {tab("submit", "Send review")}
            </nav>
          </header>
        )}

        {inReview && review && (screen === "step" || screen === "routine" || screen === "map" || screen === "files") && (
          <div className="subbar">{viewSwitch}</div>
        )}

        {screen === "inbox" && (() => {
          const all = inbox ?? [];
          const q = query.trim().toLowerCase();
          const isAddress = ADDRESS.test(query);
          const repos = [...new Set(all.map((p) => `${p.pr.owner}/${p.pr.repo}`))].sort();
          const count = (w: typeof who) => all.filter((p) => w === "all" || p.relation === w).length;
          const matches = all.filter((p) => (who === "all" || p.relation === who) && (!repoFilter || `${p.pr.owner}/${p.pr.repo}` === repoFilter)
            && (!q || isAddress || `${p.title} ${p.pr.owner}/${p.pr.repo} #${p.pr.number} ${p.author}`.toLowerCase().includes(q)));
          const list = sortBy === "big" ? [...matches].sort((a, b) => b.additions + b.deletions - (a.additions + a.deletions)) : matches;
          const groups = suggestions.filter((g) => !dismissed.includes(groupKey(g)));
          const added = fresh ? fresh.filter((p) => !all.some((x) => samePr(x.pr, p.pr))).length : 0;
          const gone = fresh ? all.filter((x) => !fresh.some((p) => samePr(x.pr, p.pr))).length : 0;
          const pick = (k: string) => setPicked(picked.includes(k) ? picked.filter((x) => x !== k) : [...picked, k]);
          const badge = (refs: string[]) => {
            const r = readiness(refs);
            return r === "ready" ? <span className="tag ok" title="Prepared earlier for these commits. Opens straight away.">Ready</span>
              : r === "stale" ? <span className="tag warn" title="There are new commits since it was prepared. Opening it prepares it again.">New commits</span> : null;
          };
          const size = (p: PrSummary) => <span className="sz"><b>+{num(p.additions)}</b> <u>−{num(p.deletions)}</u></span>;
          const crow = (p: PrSummary, inGroup = false) => {
            const k = prKey(p.pr);
            const on = picked.includes(k);
            return (
              <div className={"crow" + (inGroup ? " in" : "") + (p.relation === "mine" ? " mine" : "")} key={k}>
                <button className={"pick" + (on ? " on" : "")} aria-pressed={on} aria-label={`Select ${prLabel(p.pr)}`} onClick={() => pick(k)}><i /></button>
                <button className="open" onClick={() => open([k])} title={prLabel(p.pr)}>
                  <span className="ti">{p.title}</span>
                  <span className="rp">{inGroup || repoFilter ? prLabel(p.pr) : prShort(p.pr)}</span>
                  <span className="au">{p.author.replace(/\[bot\]$/, "")}</span>
                  <span className="tg">{badge([k])}{p.agent && <span className="tag ai">Agent</span>}{p.private && <span className="tag">{plan === "free" ? "Pro" : "Private"}</span>}</span>
                  {size(p)}
                  <span className="ag">{ago(p.updatedAt)}</span>
                </button>
              </div>
            );
          };
          const byRepo = new Map<string, PrSummary[]>();
          for (const p of list) { const k = `${p.pr.owner}/${p.pr.repo}`; byRepo.set(k, [...(byRepo.get(k) ?? []), p]); }
          return (
          <div className={screenCls}><div className="wrap wide" style={{ gap: 14 }}>
            <div className="phead">
              <div>
                <h1 className="h1">Waiting for a looksie</h1>
                <p className="mute" style={{ marginTop: 6, display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
                  <span>{inbox ? `${plural(all.length, "open pull request")}. A review can be one of them, or several that belong together.` : "A review can be one pull request, or several that belong together."}</span>
                  {inbox && (checking ? <span className="status"><span className="spin sm" />Checking for changes</span> : inboxAt ? <span className="status">Updated {ago(inboxAt) === "now" ? "just now" : ago(inboxAt) + " ago"}</span> : null)}
                </p>
              </div>
              <div className="acctwrap">
                <button className="acct" aria-expanded={acctMenu} onClick={(e) => { setMenuLeft(e.currentTarget.getBoundingClientRect().right < 360); setAcctMenu(!acctMenu); }}>
                  <span className={"face" + (active ? "" : " off")}>{(active?.login ?? "?").slice(0, 1)}</span>
                  <span className="who">{active ? active.login : accounts ? "Not connected" : "Connecting…"}<span>{active ? active.host : "GitHub"}</span></span>
                  <Ic name="down" />
                </button>
                {acctMenu && (
                  <>
                    <button className="menuveil" aria-label="Close" onClick={() => setAcctMenu(false)} />
                    <div className={"menu" + (menuLeft ? " left" : "")} role="menu">
                      {active && <><p className="mhd">Signed in as</p><div className="mrow on"><span className="face sm">{active.login.slice(0, 1)}</span><span className="who"><b>{active.login}</b><span>{active.host}</span></span><Ic name="check" /></div></>}
                      {(accounts ?? []).some((x) => !x.active && x.working) && <p className="mhd">Switch to</p>}
                      {(accounts ?? []).filter((x) => !x.active && x.working).map((x) => (
                        <button className="mrow" key={x.id} onClick={() => switchTo(api.useAccount(x.id))}><span className="face sm off">{x.login.slice(0, 1)}</span><span className="who"><b>{x.login}</b><span>{x.host}</span></span></button>
                      ))}
                      <div className="msep" />
                      <button className="mrow" onClick={() => { setAcctMenu(false); setAdding({ host: "github.com" }); }}><Ic name="plus" /><span className="who"><b style={{ fontWeight: 500 }}>Add an account</b></span></button>
                      <button className="mrow" onClick={() => { setAcctMenu(false); go("connections"); }}><Ic name="gear" /><span className="who"><b style={{ fontWeight: 500 }}>Settings</b></span></button>
                    </div>
                  </>
                )}
              </div>
            </div>

            {accounts && !active ? (
              <div className="card" style={{ maxWidth: 680, display: "flex", flexDirection: "column", gap: 14 }}>
                <h2 className="serif" style={{ fontSize: 22 }}>Connect GitHub</h2>
                <p>Looksie looks for accounts you are already signed in to on this computer, and did not find one that works.</p>
                <p><b>Easiest:</b> run <span className="mono">gh auth login</span> in a terminal, then press Look again. Looksie uses that sign-in from then on.</p>
                <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}><button className="btn sm" onClick={lookAgain}><Ic name="refresh" />Look again</button><button className="btn go sm" onClick={() => setAdding({ host: "github.com" })}><Ic name="plus" />Add an account with a token</button></div>
              </div>
            ) : (<>
            <div className="toolbar">
              <form className="search" onSubmit={(e) => { e.preventDefault(); if (isAddress) open(query.split(/[\s,]+/).filter(Boolean)); }}>
                <Ic name="search" />
                <input aria-label="Find a pull request" placeholder="Find by title, repository or author, or paste owner/repo#123 to open it" value={query} onChange={(e) => setQuery(e.target.value)} />
                {isAddress && <button className="btn go sm" type="submit" style={{ minHeight: 30 }}>Open</button>}
              </form>
              <div className="segc" role="group" aria-label="Show">
                {([["all", "All"], ["review-requested", "Waiting on you"], ["mine", "Yours"]] as const).map(([w, label]) => (
                  <button key={w} className={who === w ? "on" : ""} aria-pressed={who === w} onClick={() => setWho(w)}>{label} <i>{inbox ? count(w) : ""}</i></button>
                ))}
              </div>
              {repos.length > 1 && <select className="pill" aria-label="Repository" value={repoFilter} onChange={(e) => setRepoFilter(e.target.value)}><option value="">All repositories ({repos.length})</option>{repos.map((r) => <option key={r} value={r}>{r}</option>)}</select>}
              <select className="pill" aria-label="Sort" value={sortBy} onChange={(e) => setSortBy(e.target.value as "new" | "big")}><option value="new">Newest first</option><option value="big">Biggest first</option></select>
              <div className="segc" role="group" aria-label="View">
                <button className={view === "cards" ? "on" : ""} aria-pressed={view === "cards"} title="Cards" aria-label="Cards" onClick={() => setView("cards")}><Ic name="cards" /></button>
                <button className={view === "compact" ? "on" : ""} aria-pressed={view === "compact"} title="Compact list" aria-label="Compact list" onClick={() => setView("compact")}><Ic name="rows" /></button>
              </div>
              <button className="pill" onClick={() => refresh(active, false)} disabled={checking}><Ic name="refresh" />Refresh</button>
            </div>

            {fresh && <button className="newpill" onClick={() => { showList(fresh); setFresh(null); }}><Ic name="refresh" />{[added && `${added} new`, gone && `${gone} closed`].filter(Boolean).join(", ")} · Show</button>}
            {trialNote && (
              <div className="trialnote" role="status">
                <Ic name="badge" />
                <span>{trialNote.text}</span>
                <button className="btn go sm" onClick={() => go("plan")}>See plans</button>
                <button className="x" aria-label="Hide this note" title="Hide" onClick={() => setTrialSeen(trialNote.id)}>×</button>
              </div>
            )}
            {inboxError && (inbox ? <p className="note" style={{ color: "var(--warn)" }}>Could not check for changes: {inboxError}</p> : <div className="card warm"><b>Could not load your pull requests.</b><p style={{ marginTop: 6 }}>{inboxError}</p></div>)}

            {!inbox && !inboxError && (
              <div className="clist" aria-busy="true">
                <div className="loadrow"><span className="spin" /><span><b>Finding your pull requests</b><br /><span className="mute">Asking {active?.host ?? "GitHub"} what is waiting on you. The first time can take a few seconds.</span></span></div>
                {[0, 1, 2, 3, 4, 5].map((i) => <div className="skel" key={i} style={{ animationDelay: `${i * 0.08}s` }}><i style={{ width: `${46 + ((i * 17) % 30)}%` }} /><i /><i /></div>)}
              </div>
            )}
            {inbox?.length === 0 && <div className="card"><b>Nothing is waiting on you.</b><p className="mute" style={{ marginTop: 6 }}>Paste any pull request's address in the box above to open it.</p></div>}
            {inbox && all.length > 0 && list.length === 0 && <p className="mute">Nothing matches. <button className="linkb" onClick={() => { setQuery(""); setWho("all"); setRepoFilter(""); }}>Clear the filters</button></p>}

            {inbox && groups.length > 0 && !q && !repoFilter && who === "all" && (view === "compact" ? (
              <div className="clist">
                <div className="chead"><Ic name="down" />Looks like one piece of work <span className="n">{plural(groups.length, "group")}</span><span className="sp" /><span className="n">Suggestions, from explicit links only. You decide.</span></div>
                {groups.map((g) => {
                  const k = groupKey(g);
                  const members = g.prs.map((pr) => all.find((i) => samePr(i.pr, pr))).filter(Boolean) as PrSummary[];
                  const isOpen = openGroups.includes(k);
                  const repoCount = new Set(g.prs.map((p) => p.owner + "/" + p.repo)).size;
                  return (
                    <div key={k}>
                      <div className={"gline" + (isOpen ? "" : " closed")}>
                        <button className="tw" aria-expanded={isOpen} aria-label={isOpen ? "Hide the pull requests" : "Show the pull requests"} onClick={() => setOpenGroups(isOpen ? openGroups.filter((x) => x !== k) : [...openGroups, k])}><Ic name="down" /></button>
                        <span className="gt"><b>{g.name}</b><span>{plural(g.prs.length, "pull request")} · {plural(repoCount, "repository").replace("repositorys", "repositories")} · +{num(members.reduce((a, p) => a + p.additions, 0))} −{num(members.reduce((a, p) => a + p.deletions, 0))}</span></span>
                        <span className="why">{g.reasons[0]}</span>
                        {badge(g.prs.map(prKey))}
                        <button className="btn go sm" onClick={() => open(g.prs.map(prKey), g.name)}>Review together</button>
                        <button className="x" title="Keep them separate" aria-label="Keep them separate" onClick={() => { const d = [...dismissed, k]; setDismissed(d); store("looksie:dismissed-groups", d); }}><Ic name="cross" /></button>
                      </div>
                      {isOpen && members.map((p) => crow(p, true))}
                    </div>
                  );
                })}
              </div>
            ) : groups.map((g) => (
              <div className={"card tint gcardx" + (folded.includes(groupKey(g)) ? " folded" : "")} key={groupKey(g)} style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                <div className="chd" style={{ margin: 0 }}>
                  <h2 className="cap" style={{ color: "var(--acc)" }}>Looksie noticed</h2>
                  <span style={{ display: "flex", gap: 10, alignItems: "center" }}>
                    <span className="mute" style={{ fontSize: 12 }}>A suggestion. You decide.</span>
                    <button className="fold" aria-expanded={!folded.includes(groupKey(g))} aria-label={folded.includes(groupKey(g)) ? "Show the pull requests in this group" : "Fold this group up"} title={folded.includes(groupKey(g)) ? "Show" : "Fold up"} onClick={() => { const k = groupKey(g); setFolded(folded.includes(k) ? folded.filter((x) => x !== k) : [...folded, k]); }}><Ic name="down" /></button>
                  </span>
                </div>
                <p className="gname" style={{ fontSize: 22, display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>These {g.prs.length} look like one piece of work {badge(g.prs.map(prKey))}</p>
                {folded.includes(groupKey(g)) ? (
                  <p className="mute" style={{ marginTop: -6 }}>{g.reasons[0]} · +{num(g.prs.reduce((a, pr) => a + (all.find((i) => samePr(i.pr, pr))?.additions ?? 0), 0))} −{num(g.prs.reduce((a, pr) => a + (all.find((i) => samePr(i.pr, pr))?.deletions ?? 0), 0))}</p>
                ) : (<>
                <div>{g.reasons.map((r) => <p key={r} style={{ display: "flex", gap: 8, alignItems: "center", padding: "3px 0" }}><span className="dot ok" /><span>{r}</span></p>)}</div>
                <div style={{ background: "var(--card)", borderRadius: 16, padding: "4px 16px" }}>
                  {g.prs.map((pr) => { const x = all.find((i) => samePr(i.pr, pr)); return (
                    <div className="rowi" key={prKey(pr)} style={{ padding: "10px 0" }}><span className="l"><b>{x?.title ?? prLabel(pr)}</b><br /><span className="mute mono">{prLabel(pr)}</span></span>{x && <span className="mute">+{num(x.additions)} −{num(x.deletions)}</span>}</div>
                  ); })}
                </div>
                </>)}
                <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}>
                  <button className="btn go" onClick={() => open(g.prs.map(prKey), g.name)}>Review them together</button>
                  <button className="btn" onClick={() => { const d = [...dismissed, groupKey(g)]; setDismissed(d); store("looksie:dismissed-groups", d); }}>Keep them separate</button>
                </div>
              </div>
            )))}

            {inbox && list.length > 0 && (view === "compact" ? (
              <div className="clist">
                {[...byRepo.entries()].map(([repo, rows]) => {
                  const closed = closedRepos.includes(repo);
                  return (
                    <div key={repo}>
                      <button className={"chead" + (closed ? " closed" : "")} aria-expanded={!closed} onClick={() => setClosedRepos(closed ? closedRepos.filter((x) => x !== repo) : [...closedRepos, repo])}><Ic name="down" /><span className="mono">{repo}</span><span className="n">{rows.length}</span></button>
                      {!closed && rows.map((p) => crow(p))}
                    </div>
                  );
                })}
              </div>
            ) : (
              <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
                {list.map((p) => {
                  const k = prKey(p.pr);
                  const on = picked.includes(k);
                  return (
                    <div className="prrow" key={k}>
                      <button className={"pick" + (on ? " on" : "")} aria-pressed={on} aria-label={`Select ${prLabel(p.pr)}`} onClick={() => pick(k)}><i /></button>
                      <button className="propen" onClick={() => open([k])}>
                        <span className="t"><b>{p.title}</b><span className="mute">{prLabel(p.pr)} · {p.author}{p.agent ? " · agent-written" : ""}{p.relation === "mine" ? " · opened by you" : ""}</span></span>
                        {badge([k])}
                        {p.private && <span className="tag">{plan === "free" ? "Private · Pro" : "Private"}</span>}
                        <span className="mute">+{num(p.additions)} −{num(p.deletions)}</span>
                      </button>
                    </div>
                  );
                })}
              </div>
            ))}
            </>)}
          </div>
          {picked.length > 0 && (
            <div className="dock" role="region" aria-label="Ticked pull requests">
              <span style={{ padding: "0 8px 0 14px", fontWeight: 500 }}>{picked.length} ticked</span>
              <button className="btn" onClick={() => setPicked([])}>Clear</button>
              <button className="btn go" disabled={picked.length < 2} onClick={() => open(picked)}>{picked.length < 2 ? "Tick one more to review together" : `Review ${picked.length} together`}<Ic name="next" /></button>
            </div>
          )}
          </div>
          );
        })()}

        {screen === "connections" && (
          <div className={screenCls}><div className="wrap" style={{ maxWidth: 1180 }}>
            <div className="phead">
              <div><h1 className="h1">Settings</h1><p className="mute" style={{ marginTop: 6 }}>Your accounts and your AI. Looksie finds them for you; change anything here.</p></div>
              <button className="pill" onClick={() => { setAccounts(null); lookAgain(); api.aiSetup().then(setAi).catch(() => undefined); api.agentStates().then(setAgents).catch(() => undefined); }}><Ic name="refresh" />Look again</button>
            </div>
            <div className="cols">
              <div className="grow">
                <div className="card csec">
                  <div className="chd"><h2 className="cap">GitHub accounts</h2><button className="btn sm" onClick={() => setAdding({ host: "github.com" })}><Ic name="plus" />Add an account</button></div>
                  {!accounts && <p className="mute" style={{ display: "flex", gap: 10, alignItems: "center" }}><span className="spin sm" />Looking for accounts on this computer…</p>}
                  {accounts?.length === 0 && <p className="mute">None found yet. Sign in with <span className="mono">gh auth login</span> in a terminal and press Look again, or add one with a token.</p>}
                  {accounts?.map((x) => (
                    <div className="arow" key={x.id}>
                      <span className={"face" + (x.active ? "" : " off")}>{x.login.slice(0, 1)}</span>
                      <div className="who"><b>{x.login}</b><span>{x.host} · {x.sourceText}</span></div>
                      {x.working ? <span className="st ok"><span className="dot ok" />Working</span> : <span className="st bad" title={x.problem ?? ""}><span className="dot bad" />{x.problem ?? "Not working"}</span>}
                      <div className="acts">
                        {x.active ? <span className="tag ai">In use</span> : x.working && <button className="btn sm" onClick={() => switchTo(api.useAccount(x.id))}>Use this</button>}
                        {x.source === "saved" && !x.working && <button className="btn sm" onClick={() => setAdding({ host: x.host })}>Replace token</button>}
                        {x.source === "saved" && <button className="btn sm" onClick={() => switchTo(api.removeAccount(x.id))}>Remove</button>}
                      </div>
                    </div>
                  ))}
                  <p className="note" style={{ marginTop: 8 }}>Your sign-ins stay where they are. Looksie borrows the GitHub CLI's, or keeps a token you add in your keychain. Using a company GitHub? Add it with its address.</p>
                </div>
                <div className="card csec">
                  <div className="chd"><h2 className="cap">More places, coming soon</h2></div>
                  <div className="soon"><span>Azure DevOps</span><span>GitLab</span><span>Bitbucket</span><span>A folder on this computer</span></div>
                  <p className="mute" style={{ fontSize: 13, marginTop: 10 }}>Same on Free and Pro.</p>
                </div>
              </div>
              <div className="aside" style={{ flex: "1 1 380px" }}>
                <div className="card csec">
                  <div className="chd"><h2 className="cap">Your AI</h2><span className="tag">Optional</span></div>
                  <p className="mute" style={{ fontSize: 13.5 }}>AI adds a plain-English summary, a question for each decision and checks of the author's claims. Two ways to add it; Looksie works fine without either.</p>
                  {!ai ? <p className="mute" style={{ display: "flex", gap: 10, alignItems: "center" }}><span className="spin sm" />Reading your AI settings…</p> : (<>
                    <div className="aiway">
                      <p className="cap">1 · Your own API key</p>
                      <div className="company" role="radiogroup" aria-label="AI company">
                        {ai.providers.map((p) => (
                          <button key={p.id} role="radio" aria-checked={ai.provider === p.id} className={ai.provider === p.id ? "on" : ""} onClick={() => { const first = ai.models.find((m) => m.provider === p.id); if (first && ai.provider !== p.id) api.setAiModel(first.id).then(setAi).catch(() => undefined); }}>
                            {p.hasKey && <span className="dot ok" title="Key saved" />}{p.name}
                          </button>
                        ))}
                      </div>
                      {aiCompany && <AiKey company={aiCompany} onChange={setAi} />}
                      {ai.hasKey && (<>
                        <div className="models" role="radiogroup" aria-label="Model">
                          {ai.models.filter((m) => m.provider === ai.provider).map((m) => (
                            <button key={m.id} role="radio" aria-checked={ai.model === m.id} className={"model" + (ai.model === m.id ? " on" : "")} onClick={() => api.setAiModel(m.id).then(setAi).catch(() => undefined)}>
                              <b>{m.name}</b><span className="mute">{m.note}</span>
                            </button>
                          ))}
                        </div>
                        <div className="setlist" style={{ marginTop: 4 }}>
                          <Switch on={aiAuto} onChange={setAiAuto} label="Summarise new reviews automatically" hint="Each one is a small charge on your API key." />
                        </div>
                      </>)}
                    </div>
                    <div className="aiway">
                      <p className="cap">2 · Your own Claude Code or Codex</p>
                      <p className="mute" style={{ fontSize: 13.5 }}>Connect it once to do the whole review in chat, or to have it write the AI help on your own plan. Looksie gives it a review skill and its tools.</p>
                      {!agents ? <p className="mute" style={{ display: "flex", gap: 10, alignItems: "center" }}><span className="spin sm" />Looking for Claude Code and Codex…</p> : agents.map((a) => <AgentRow key={a.id} agent={a} onChange={setAgents} />)}
                      {agents && (
                        <details className="agentway">
                          <summary>Connect by hand instead</summary>
                          {agents.map((a) => a.manual.map((m, i) => <Copyable key={a.id + i} label={i === 0 ? a.name : `${a.name}, then`} text={m} />))}
                          <p className="mute" style={{ fontSize: 12.5 }}>By hand, Codex gets Looksie's tools but not its skill: ask it to "use the looksie tools" instead.</p>
                        </details>
                      )}
                    </div>
                  </>)}
                </div>
              </div>
            </div>
          </div></div>
        )}

        {screen === "plan" && shop && (
          <div className={screenCls}><div className="wrap" style={{ maxWidth: 980 }}>
            {pending ? (
              <div><h1 className="h1">That one is in a private repository</h1><p className="mute" style={{ marginTop: 6 }}>Private repositories are part of Looksie Pro. {shop.status.trialUsed ? "Your free trial has finished." : `Try it free for ${shop.trialDays} days: no card, no account.`}</p></div>
            ) : (
              <div><h1 className="h1">{plan === "pro" ? "You are on Pro" : plan === "trial" ? `Your trial has ${plural(shop.status.trialDaysLeft ?? 0, "day")} left` : shop.status.trialUsed ? "Your free trial has finished" : "Free, and Pro when you need it"}</h1><p className="mute" style={{ marginTop: 6 }}>{plan === "free" && shop.status.trialUsed ? "Nothing is lost: public repositories stay free, for good. Pro opens private ones again." : "Looksie is free on public repositories, for good. Pro adds private ones. Every account and AI tool works the same on both."}</p></div>
            )}
            {shop.status.checkOverdue && <p className="note" style={{ color: "var(--warn)" }}>Your licence key ending {shop.status.keyHint} has not been checked with the store for over a month. Connect to the internet and Looksie will check it, and Pro comes straight back.</p>}
            <div className="cols" style={{ alignItems: "stretch", maxWidth: 980 }}>
              <div className="card" style={{ flex: "1 1 340px", display: "flex", flexDirection: "column", gap: 12 }}>
                <div className="chd" style={{ margin: 0 }}><h2 className="cap">Free</h2>{plan === "free" && <span className="tag">Your plan</span>}</div>
                <p className="gname" style={{ fontSize: 26 }}>$0</p>
                <ul className="ticks"><li>Every public repository</li><li>Every connection: GitHub today, more as they arrive</li><li>The whole walkthrough: briefing, decisions, files, send review</li><li>The AI layer, with your own API key or AI agent</li><li>Groups of pull requests</li></ul>
              </div>
              <div className="card" style={{ flex: "1 1 340px", display: "flex", flexDirection: "column", gap: 12, boxShadow: "0 0 0 2px var(--acc), var(--shadow)" }}>
                <div className="chd" style={{ margin: 0 }}><h2 className="cap" style={{ color: "var(--acc)" }}>Pro</h2>{plan !== "free" && <span className="tag ai">{plan === "pro" ? "Your plan" : "On trial"}</span>}</div>
                <p className="gname" style={{ fontSize: 26 }}>{shop.priceMonthly} a month <span className="mute" style={{ fontSize: 15, fontWeight: 400 }}>or {shop.priceYearly} a year, 20% off</span></p>
                <ul className="ticks"><li>Everything in Free</li><li><b>Private repositories</b>, on every connection</li></ul>
                <p className="cap" style={{ marginTop: 4 }}>Coming to Pro*</p>
                <ul className="ticks soon"><li><b>Ask AI about a decision:</b> talk a change through before you choose</li><li><b>Your own rules:</b> checks for the things your team cares about</li></ul>
                {offer && offer.left > 0 && plan !== "pro" && <p style={{ background: "var(--acc-soft)", color: "var(--acc)", borderRadius: 14, padding: "10px 14px", fontWeight: 500 }}>{shop.offer}{offer.left < offer.max ? ` ${offer.left} of ${offer.max} left.` : ""}</p>}
                {plan === "pro" ? (
                  <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                    <p className="mute">Licence key ending {shop.status.keyHint}.</p>
                    <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                      <button className="tag ext" onClick={() => api.openUrl(shop.portal)}>Manage your subscription<Ic name="external" /></button>
                      <button className="tag" title="Frees one of your 3 devices for another computer" onClick={() => api.removeLicence().then(setShop)}>Remove it from this computer</button>
                    </div>
                  </div>
                ) : (
                  <PlanActions shop={shop} onChange={gotPro} />
                )}
              </div>
            </div>
            <p className="mute" style={{ fontSize: 12 }}>* Planned, and subject to change. You pay for what Pro does today. Paying is handled by our payment partner, who sends your licence key by email. Looksie has no accounts and collects nothing about you.</p>
          </div></div>
        )}

        {screen === "loading" && (
          <div className={screenCls}><div className="wrap" style={{ maxWidth: 620, minHeight: "100%", justifyContent: "center", gap: 0 }}>
            <Logo looking={!loadError} size={72} />
            <h1 className="h1" style={{ marginTop: 20 }}>{loadError ? "That did not work" : "Having a looksie"}</h1>
            {loadError ? (
              <><p style={{ marginTop: 8 }}>{loadError}</p><p style={{ marginTop: 16 }}><button className="btn" onClick={() => go("inbox")}><Ic name="back" />Back to the inbox</button></p></>
            ) : (
              <><p className="mute" style={{ marginTop: 6 }}>{loadingWhat.join(", ")}</p>
                <div className="checks">
                  {["Reading the pull request and its linked issues", "Fetching the changed files", "Working out what depends on what", "Grouping the changes into steps", "Checking the description against the code"].map((t, i) => <span key={t} className={"d" + (i + 1)}><Ic name="check" />{t}</span>)}
                </div>
                <div className="pbar wait"><i /></div></>
            )}
          </div></div>
        )}

        {screen === "briefing" && review && (
          <div className={screenCls}><div className="wrap">
            <div>
              <p className="mute">{plural(review.prs.length, "pull request")} · {plural(review.files.reduce((a, f) => a + f.additions + f.deletions, 0), "changed line")} · {plural(review.files.length, "file")}{review.prs.some((p) => p.summary.agent) && <> · <span className="tag on-card">Agent-written</span></>}</p>
              <h1 className="h1" style={{ marginTop: 8 }}>{review.briefing.title}</h1>
            </div>
            {moved && <div className="card warm" style={{ padding: "12px 18px" }}><b>This has new commits since you last looked.</b> Your earlier answers and draft comments were for the older version, so this is a clean sheet.</div>}
            <div className="cols">
              <div className="grow">
                <div className="card"><div className="chd"><h2 className="cap">What and why</h2><span className="mute" style={{ fontSize: 12 }}>{layer?.summary ? "Read this before the steps." : "In the author's own words. Read this before the steps."}</span></div>
                  {layer?.summary && <p style={{ fontSize: 18, lineHeight: 1.55, margin: "6px 0 14px" }}>{layer.summary} <span className="tag ai"><Ic name="spark" />Written by AI</span></p>}
                  {review.briefing.descriptions.map((d) => (
                    <div className="rowi" key={prKey(d.pr)} style={{ justifyContent: "flex-start", alignItems: "flex-start" }}><span className="tag mono">{prShort(d.pr)}</span><span className="l"><Folded text={d.text} /></span></div>
                  ))}
                </div>
                {(review.briefing.fits.length > 0 || review.briefing.headline) && (
                  <div className="card"><div className="chd"><h2 className="cap">How the pull requests fit together</h2><button className="tag" onClick={() => go("map")}>See the map</button></div>
                    {review.briefing.headline && <p style={{ fontSize: 17, lineHeight: 1.5, margin: "4px 0 10px" }}>{review.briefing.headline}</p>}
                    {review.briefing.fits.map((f) => <div className="rowi" key={f.text}><span className="l">{f.text}</span><span className="tag">Found by a rule</span></div>)}
                  </div>
                )}
                <div className="card"><div className="chd"><h2 className="cap">What the author says</h2><span className="mute" style={{ fontSize: 12 }}>Each claim checked against the code and CI</span></div>
                  {review.briefing.claims.length === 0 && <p className="mute">No checkable claims found in the description.</p>}
                  {review.briefing.claims.map((c, i) => {
                    const si = steps.findIndex((s) => s.id === c.stepId);
                    const byAi = c.status === "unchecked" ? layer?.claims.find((x) => x.index === i) : undefined;
                    if (byAi) c = { ...c, status: byAi.status, note: byAi.note };
                    return (
                      <div className={"rowi claim " + CLAIM_CLASS[c.status]} key={i}><span className="l">{c.text} {review.prs.length > 1 && <span className="mute mono">{prShort(c.pr)}</span>}</span>
                        <span style={{ display: "flex", gap: 8, alignItems: "center" }}><span className="st">{c.note}</span>{byAi && <span className="tag ai">AI</span>}{si >= 0 && <button className="tag" onClick={() => goStep(si)}>Open it</button>}</span></div>
                    );
                  })}
                </div>
                {review.briefing.notMentioned.length > 0 && (
                  <div className="card"><h2 className="cap">Not mentioned in the description</h2>
                    {review.briefing.notMentioned.map((f) => <div className="rowi" key={f.text}><span className="l">{f.text}</span><span className="tag">Found by a rule</span></div>)}
                  </div>
                )}
              </div>
              <div className={"aside side sidepanel" + sideFade} ref={sideRef} onScroll={sideEdges}>
                {steps.length > 0 && (
                  <div className="card nextup">
                    <p className="cap">Next</p>
                    <p className="gname" style={{ fontSize: 18 }}>{total > 0 ? `${plural(total, "decision")} to make` : "Nothing needs a decision"}{routine.length > 0 && <span className="mute" style={{ fontSize: 14, fontWeight: 400 }}>{` · ${plural(routine.length, "quick look")}`}</span>}</p>
                    <button className="btn go" style={{ justifyContent: "center", minHeight: 48 }} onClick={() => (total > 0 ? goStep(0) : go("routine"))}>{answered > 0 && answered < total ? `Carry on: ${answered} of ${total} answered` : "Start the walkthrough"}<Ic name="next" /></button>
                    {overLimit > 0 && (
                      <p className="mute" style={{ fontSize: 13, marginTop: 4 }}>
                        {lifted
                          ? `Includes ${plural(overLimit, "step")} over the usual limit for a review this size. `
                          : `${plural(overLimit, "more step")} scored like ${overLimit === 1 ? "a decision" : "decisions"} but ${overLimit === 1 ? "is" : "are"} over the limit for a review this size. ${overLimit === 1 ? "It comes" : "They come"} first in the quick looks. `}
                        <button className="linkb" onClick={() => { setAllIn(lifted ? null : review.id); setStepIdx(0); }}>{lifted ? "Back to the limit" : overLimit === 1 ? "Make it a decision too" : "Make them decisions too"}</button>
                      </p>
                    )}
                  </div>
                )}
                {review.briefing.needsYou.length > 0 && (() => {
                  // Many secrets or settings for one repository read as one item with their names, not a line each.
                  const groups: { head: string; names: string[] }[] = [];
                  const plain: string[] = [];
                  for (const f of review.briefing.needsYou) {
                    const sec = f.text.match(/^Check the secret (\S+) is set in (.+)\.$/);
                    const env = f.text.match(/^Check (\S+) is set wherever (.+) runs\.$/);
                    const head = sec ? `Secrets to set in ${sec[2]}` : env ? `Settings needed wherever ${env[2]} runs` : null;
                    if (!head) { plain.push(f.text); continue; }
                    const g = groups.find((x) => x.head === head) ?? (groups.push({ head, names: [] }), groups[groups.length - 1]);
                    const name = (sec ?? env)![1];
                    if (!g.names.includes(name)) g.names.push(name);
                  }
                  const SHOW = 5;
                  const shown = allNeeds ? plain : plain.slice(0, SHOW);
                  return (
                    <div className="card warm needs"><h2 className="cap" style={{ color: "var(--warn)" }}>Needs you</h2>
                      {groups.map((g) => g.names.length === 1
                        ? <p key={g.head}>{g.head.startsWith("Secrets") ? `Check the secret ${g.names[0]} is set in ${g.head.slice(15)}.` : `Check ${g.names[0]} is set wherever ${g.head.slice(25, -5)} runs.`}</p>
                        : <div key={g.head}><p>{g.head} ({g.names.length})</p><div className="names">{g.names.map((n) => <span key={n} className="mono">{n}</span>)}</div></div>)}
                      {shown.map((t) => <p key={t}>{t}</p>)}
                      {plain.length > SHOW && <button className="tag" onClick={() => setAllNeeds(!allNeeds)}>{allNeeds ? "Show fewer" : `Show ${plain.length - SHOW} more`}</button>}
                    </div>
                  );
                })()}
                <div className="card" style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                  <div className="chd" style={{ margin: 0 }}><h2 className="cap">AI layer</h2>{aiLayer && <span className="tag ai"><Ic name="spark" />{aiLayer.provider ? `From ${aiLayer.provider}` : "Added"}</span>}</div>
                  {!aiLayer && !aiBusy && !agentWait && <p className="mute">A plain-English summary, a question for each decision, and a check of the author's claims. {canAsk ? `${modelName} is given the description and the changes for the decisions, nothing else.${aiCost !== null ? ` About $${aiCost < 0.01 ? "0.01" : aiCost.toFixed(2)} on your API key.` : ""}` : "Add your own API key in Settings, or use your own Claude Code or Codex."}</p>}
                  {aiBusy && (() => { const p = aiProgress(); return (
                    <div className="aiprog" role="status">
                      <p style={{ display: "flex", gap: 10, alignItems: "center" }}><span className="spin sm" /><b>{p.stage}</b></p>
                      <div className="pbar slim"><i style={{ width: p.pct + "%" }} /></div>
                      <p className="mute" style={{ fontSize: 13 }}>{p.left}. You can keep reading; it fills in when it is done.</p>
                    </div>
                  ); })()}
                  {agentWait && !aiLayer && <p className="aiprog" role="status" style={{ display: "flex", gap: 10, alignItems: "center" }}><span className="spin sm" /><span>Waiting for your agent's answer. It appears here as soon as it is saved.</span></p>}
                  {aiLayer && !aiBusy && <p className="mute">{aiLayer.summary ? "Summary written. " : ""}{plural(aiLayer.steps.length, "question")}, {plural(aiLayer.claims.length, "claim")} checked.{aiLayer.dropped > 0 ? ` ${plural(aiLayer.dropped, "answer")} thrown away: no real line quoted, or not a step in this review.` : ""}{aiOn ? "" : " The AI layer is switched off, so it is hidden."}</p>}
                  {aiError && <p style={{ color: "var(--bad)" }}>{aiError}</p>}
                  <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                    {canAsk
                      ? <button className="btn go sm" disabled={aiBusy} onClick={askAi}><Ic name="spark" />{aiLayer ? "Ask again" : `Ask ${aiCompany?.assistant ?? "AI"}`}</button>
                      : <button className="btn sm" onClick={() => go("connections")}><Ic name="gear" />Set up AI</button>}
                    <button className="btn sm" onClick={async () => setPromptText(await api.aiPrompt(review))}>Show what will be sent</button>
                  </div>
                  {ai && (
                    <details className="agentway">
                      <summary>Review it in Claude Code or Codex instead</summary>
                      {(() => {
                        const on = (agents ?? []).filter((a) => a.connected && !a.elsewhere);
                        return on.length > 0
                          ? <>
                              <p style={{ fontSize: 14 }}>Type this in {on.map((a) => a.name).join(" or ")}. It picks up the review open here: go through the decisions in chat, ask about the code, and post when you are ready. Answers show up here too.</p>
                              {on.map((a) => <Copyable key={a.id} label={`In ${a.name}`} text={a.command} />)}
                              <p className="mute" style={{ fontSize: 12.5 }}>Or have it write the AI help shown here (a summary and a question per decision), on your own plan:</p>
                              <Copyable label={`Ask ${on[0].name}`} text="Use the looksie tools to write Looksie's AI help for the review open in Looksie." onCopy={() => setAgentWait(Date.now())} />
                            </>
                          : <p style={{ fontSize: 14 }}>Connect Claude Code or Codex once in <button className="linkb" onClick={() => go("connections")}>Settings, Your AI</button>. Then you can do the whole review in chat, and your answers show up here too.</p>;
                      })()}
                    </details>
                  )}
                </div>
                <div className="card"><div className="chd"><h2 className="cap">In this review</h2></div>
                  {review.prs.map((p) => (
                    <div className="rowi" key={prKey(p.summary.pr)} style={{ padding: "10px 0" }}>
                      <span className="l"><b>{p.summary.title}</b><br /><span className="mute mono">{prLabel(p.summary.pr)}</span><br />
                        <span className="mute" style={{ fontSize: 12 }}>+{p.summary.additions} −{p.summary.deletions} · {p.ci.unknown ? "CI results could not be read" : p.ci.total === 0 ? "no CI run on this commit" : p.ci.failed > 0 ? `${p.ci.failed} of ${p.ci.total} CI checks failed` : `${p.ci.total} CI checks, none failed`} · into {p.baseRef}</span></span>
                      <Ext url={p.summary.url} what="Open" host={p.summary.pr.host} />
                    </div>
                  ))}
                  <p className="mute" style={{ fontSize: 12, marginTop: 8 }}>{review.briefing.sources.join(" · ")}</p>
                </div>
                {review.skipped.length > 0 && (
                  <div className="card"><h2 className="cap">Skipped · safe to ignore</h2><p className="mute" style={{ marginTop: 6 }}>{plural(review.skipped.length, "file")}, {plural(review.skipped.reduce((a, s) => a + s.lines, 0), "line")}. They are still in the Files tab.</p></div>
                )}
              </div>
            </div>
          </div></div>
        )}

        {screen === "map" && review && cur && (
          <div className={screenCls}><div className="wrap" style={{ maxWidth: 1360 }}>
            <div className="phead"><div><h1 className="h1">How the decisions fit together</h1><p className="mute" style={{ marginTop: 6 }}>Left to right by layer, one row per pull request. Lines show what builds on what. Pick a card to see it, then open it.{hidden > 0 && ` There are too many connections to draw at once, so only the selected card's are shown (${num(hidden)} more).`}</p></div></div>
            <div className="cols">
              <div className="grow"><div className="scrollx">
                <div className="matrix" ref={matrixRef} style={{ gridTemplateColumns: `repeat(${LAYERS.filter((l) => steps.some((s) => s.layer === l.id)).length}, minmax(200px, 1fr))` }}>
                  {LAYERS.filter((l) => steps.some((s) => s.layer === l.id)).map((l) => <div key={l.id} className="colhd2">{l.title}</div>)}
                  {review.prs.map((p) => LAYERS.filter((l) => steps.some((s) => s.layer === l.id)).map((l, li) => (
                    <div key={prKey(p.summary.pr) + l.id} className="cell">
                      {li === 0 && (() => {
                        // One owner for the whole review: leave it out. Long names are cut with "…", the number always shows, and the full name is on hover.
                        const oneOwner = review.prs.every((x) => x.summary.pr.owner === p.summary.pr.owner);
                        const name = oneOwner ? p.summary.pr.repo : `${p.summary.pr.owner}/${p.summary.pr.repo}`;
                        return <span className="lanelabel" title={`${prLabel(p.summary.pr)}: ${p.summary.title}`}><span className="ln-name">{name}</span><span className="ln-num">#{p.summary.pr.number}</span></span>;
                      })()}
                      {steps.map((s, i) => s.layer === l.id && s.files.length > 0 && samePr(s.files[0].pr, p.summary.pr) ? (
                        <button key={s.id} data-id={s.id} className={"mnode " + stateOf(s) + (s.tier === "glance" ? " glance" : "") + (i === stepIdx ? " cur" : "")} aria-pressed={i === stepIdx} onClick={() => setStepIdx(i)}>
                          <span className="k">{s.tier === "glance" ? "Quick look" : `Decision ${i + 1}`}</span><b>{s.title}</b><span className="mute" style={{ fontSize: 12 }}>{markOf(s)}</span>
                        </button>
                      ) : null)}
                    </div>
                  )))}
                  <svg className="medges" aria-hidden="true">
                    <defs>
                      <marker id="mh" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse"><path className="tip" d="M0,0 L10,5 L0,10 z" /></marker>
                      <marker id="mhh" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse"><path className="tip hot" d="M0,0 L10,5 L0,10 z" /></marker>
                    </defs>
                    {edges.map((e, k) => <path key={k} d={e.d} className={e.hot ? "hot" : ""} markerEnd={e.hot ? "url(#mhh)" : "url(#mh)"} />)}
                  </svg>
                </div>
              </div></div>
              <div className="aside side" style={{ position: "sticky", top: 16, alignSelf: "flex-start" }}>
                <div className="card" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                  <div className="chd" style={{ margin: 0 }}><h2 className="cap">{curIsRoutine ? "Quick look" : `Decision ${stepIdx + 1} of ${total}`}</h2><span className={"mark " + stateOf(cur)}>{markOf(cur)}</span></div>
                  <p className="gname">{cur.title}</p>
                  <p style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>{prLinks(cur)}</p>
                  <p>{cur.question ?? (curIsRoutine ? "A quick look should do." : noRule(cur))}</p>
                  {cur.reasons.length > 0 && <ul className="facts">{cur.reasons.map((r) => <li key={r}>{r}</li>)}</ul>}
                  <Links step={cur} steps={steps} stateOf={stateOf} onPick={setStepIdx} />
                  <button className="btn go" style={{ justifyContent: "center" }} onClick={() => goStep(stepIdx)}>Open this decision<Ic name="next" /></button>
                </div>
              </div>
            </div>
          </div></div>
        )}

        {screen === "step" && review && cur && (
          <>
            <div className={screenCls}><div className="wrap">
              <div style={{ display: "flex", justifyContent: "space-between", gap: 12, flexWrap: "wrap", alignItems: "center" }}>
                <p className="mute">{curIsRoutine ? `Quick look ${stepIdx - total + 1} of ${routine.length}` : `Decision ${stepIdx + 1} of ${total}`} · {LAYERS.find((l) => l.id === cur.layer)?.title} · <span style={{ display: "inline-flex", gap: 6, verticalAlign: "middle" }}>{prLinks(cur)}</span></p>
                {flagOf(cur) !== "none" && <p><span className={"flag " + flagOf(cur)}>{FLAG_TEXT[flagOf(cur)]}</span> <span className="mute" style={{ fontSize: 12 }}>{flagOf(cur) === cur.flag ? "flagged by a rule" : "flagged by AI"}</span></p>}
              </div>
              <div className={"glide " + (dir > 0 ? "f" : "b") + anim} key={cur.id}>
                <h1 className="h1" style={{ fontSize: 28 }}>{cur.title}</h1>
                <div className="card" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
                  <div className="chd" style={{ margin: 0 }}><h2 className="cap">Your call</h2>{aiStep(cur) ? <span className="tag ai"><Ic name="spark" />Asked by AI</span> : <span className="tag">{cur.question ? "Asked by a rule" : "Facts only"}</span>}</div>
                  <p className="q">{aiStep(cur)?.question ?? cur.question ?? (curIsRoutine ? "A quick look should do. Glance over it, then move on." : noRule(cur))}</p>
                  {cur.reasons.length > 0 && <div><p className="cap" style={{ marginBottom: 6, display: "flex", alignItems: "center", gap: 6 }}>{curIsRoutine ? "Why it's a quick look" : "Why this needs you"}<button className="info" aria-label="How Looksie decides" title="How Looksie decides" onClick={() => setHow(true)}><Ic name="info" /></button></p><ul className="facts">{cur.reasons.map((r) => <li key={r} className={curIsRoutine ? "" : "why"}>{r}</li>)}</ul></div>}
                  <ul className="facts">{cur.facts.map((f) => <li key={f.text}>{f.text}</li>)}</ul>
                  {aiStep(cur)?.why && <div className="aibox"><Ic name="spark" /><p>{aiStep(cur)!.why}</p></div>}
                  {aiStep(cur) && cur.question && <p className="mute" style={{ fontSize: 13 }}>A rule also asks: {cur.question}</p>}
                  <div className="vrow">
                    {(["fine", "change", "question"] as Answer[]).map((a) => (
                      <button key={a} className={"vbtn " + a + (saved.answers[cur.id] === a ? " on" : "")} aria-pressed={saved.answers[cur.id] === a}
                        onClick={() => setSaved((v) => ({ ...v, answers: { ...v.answers, [cur.id]: v.answers[cur.id] === a ? undefined : a } }))}>
                        <Ic name={a === "fine" ? "check" : a === "change" ? "cross" : "ask"} />{a === "question" ? "I have a question" : ANSWER_TEXT[a]}
                      </button>
                    ))}
                  </div>
                  <label className="mute">Note for the author (becomes a review comment)
                    <textarea value={saved.notes[cur.id] ?? ""} onChange={(e) => setSaved((v) => ({ ...v, notes: { ...v.notes, [cur.id]: e.target.value } }))} />
                  </label>
                </div>
                <div className="card" style={{ padding: "14px 24px", display: "flex", flexWrap: "wrap", gap: "10px 14px", alignItems: "center" }}>
                  <Links step={cur} steps={steps} stateOf={stateOf} onPick={goStep} inline />
                  <span style={{ flex: 1 }} />
                  <button className="tag" onClick={() => go("map")}><Ic name="grid" />See it on the map</button>
                </div>
                {cur.files.length > 0 && (() => {
                  const keys = cur.files.map((sf) => `${cur.id}|${prKey(sf.pr)}|${sf.path}`);
                  const allShut = keys.every((k) => shut.includes(k));
                  const many = cur.files.length > 1;
                  return (
                    <div className="diffnav" role="toolbar" aria-label="Files in this decision">
                      <span className="cap">{plural(cur.files.length, "file")}</span>
                      {diffSwitch}
                      <span style={{ flex: 1 }} />
                      {many && <button className="tag" onClick={() => setShut(allShut ? shut.filter((k) => !keys.includes(k)) : [...new Set([...shut, ...keys])])}>{allShut ? "Unfold all" : "Fold all"}</button>}
                      {many && <button className="tag" title="Previous file" onClick={() => jumpDiff(-1)}>↑ Previous</button>}
                      {many && <button className="tag" title="Next file" onClick={() => jumpDiff(1)}>↓ Next</button>}
                      {many && <button className="tag" title="Back to the top" onClick={() => jumpDiff(0)}>Top</button>}
                    </div>
                  );
                })()}
                {cur.files.map((sf) => {
                  const f = fileOf(sf.pr, sf.path);
                  if (!f) return null;
                  const k = `${cur.id}|${prKey(sf.pr)}|${sf.path}`;
                  const isShut = shut.includes(k);
                  const fold = <button className="fold" aria-expanded={!isShut} aria-label={isShut ? `Show the changes to ${sf.path}` : `Fold up ${sf.path}`} title={isShut ? "Show" : "Fold up"} onClick={() => setShut(isShut ? shut.filter((x) => x !== k) : [...shut, k])}><Ic name="down" /></button>;
                  return (
                    <div className={"diff scrollx" + (isShut ? " shut" : "")} data-diff key={prKey(sf.pr) + sf.path}>
                      <FileHead file={f} lead={fold}><button className="tag on-card" onClick={() => openFile(f)}>Open in Files</button><Ext url={f.url} what="Whole file" host={f.pr.host} onCard /></FileHead>
                      {!isShut && sf.focus && <p className="mute" style={{ padding: "0 22px 8px", fontSize: 12 }}>Showing only the lines that mention {sf.focus.join(", ")}.</p>}
                      {!isShut && <Diff file={f} focus={sf.focus} saved={saved} onComment={setComment} split={split} threads={review.threads.filter((t) => samePr(t.pr, f.pr))} />}
                    </div>
                  );
                })}
                <p className="mute" style={{ fontSize: 12, marginTop: -8 }}>Hover a line and press + to comment on it.</p>
              </div>
            </div></div>
            <div className="dock">
              {curIsRoutine ? (
                <button className="btn" onClick={() => (stepIdx > total ? goStep(stepIdx - 1) : go("routine"))}><Ic name="back" />{stepIdx > total ? "Back" : "Quick looks"}</button>
              ) : (
                <button className="btn" onClick={() => (stepIdx > 0 ? goStep(stepIdx - 1) : go("briefing"))}><Ic name="back" />{stepIdx > 0 ? "Back" : "Briefing"}</button>
              )}
              <button className="tag" onClick={() => setOutline(true)}>{curIsRoutine ? `Quick look ${stepIdx - total + 1} of ${routine.length}` : `Decision ${stepIdx + 1} of ${total}`} · All steps</button>
              {!curIsRoutine && total <= 12 && <div className="segs" role="group" aria-label="Decisions">{decisions.map((s, i) => <button key={s.id} className={"seg " + stateOf(s) + (i === stepIdx ? " cur" : "")} aria-label={`Decision ${i + 1}: ${s.title}`} onClick={() => goStep(i)} />)}</div>}
              {curIsRoutine ? (
                <button className="btn go" onClick={() => { setSaved((v) => ({ ...v, answers: { ...v.answers, [cur.id]: v.answers[cur.id] ?? "fine" } })); if (stepIdx < steps.length - 1) goStep(stepIdx + 1); else go("routine"); }}>Looks fine<Ic name="next" /></button>
              ) : (
                <button className="btn go" onClick={() => (stepIdx < total - 1 ? goStep(stepIdx + 1) : go(routine.length ? "routine" : "submit"))}>{stepIdx < total - 1 ? "Next decision" : routine.length ? "Quick looks (optional)" : "Send review"}<Ic name="next" /></button>
              )}
            </div>
          </>
        )}

        {screen === "routine" && review && (
          <div className={screenCls}><div className="wrap">
            <div style={{ display: "flex", justifyContent: "space-between", gap: 16, flexWrap: "wrap", alignItems: "flex-end" }}>
              <div><p className="mute">{answered === total ? "All decisions answered." : `${answered} of ${total} decisions answered.`} Optional from here.</p><h1 className="h1" style={{ marginTop: 4 }}>Quick looks</h1><p className="mute" style={{ marginTop: 6 }}>{overLimit > 0 && !lifted ? "Looksie thinks these do not need a decision, or the review already has as many as its size calls for." : "Looksie thinks these do not need a decision."} Each says why. Glance at any you like, or go straight to sending. <button className="linkb" onClick={() => setHow(true)}>How Looksie decides</button></p></div>
              <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}>
                <button className="btn" onClick={() => goStep(Math.max(0, Math.min(stepIdx, total - 1)))}><Ic name="back" />Decisions</button>
                <button className="btn" onClick={() => setSaved((v) => ({ ...v, answers: { ...v.answers, ...Object.fromEntries(routine.filter((s) => !v.answers[s.id]).map((s) => [s.id, "fine" as Answer])) } }))}>Mark all as looked at</button>
                <button className="btn go" onClick={() => go("submit")}>Send review<Ic name="next" /></button>
              </div>
            </div>
            {routine.map((s, k) => {
              const i = steps.indexOf(s);
              const lines = s.files.reduce((a, f) => a + f.additions + f.deletions, 0);
              // When some steps are over the limit, they come first under their own heading, then the routine ones.
              const head = routine.some((x) => x.overLimit) && (k === 0 || !!routine[k - 1].overLimit !== !!s.overLimit)
                ? (s.overLimit ? `Over the limit: ${plural(routine.filter((x) => x.overLimit).length, "step")} that scored like decisions, strongest first` : "Routine changes")
                : null;
              return (
                <Fragment key={s.id}>{head && <p className="cap" style={{ marginTop: 18 }}>{head}{s.overLimit && <> · <button className="linkb" onClick={() => { setAllIn(review.id); goStep(total); }}>Make them decisions too</button></>}</p>}
                <div className="dcard" style={{ display: "flex", flexWrap: "wrap", gap: "10px 16px", alignItems: "center" }}>
                  <span style={{ flex: "1 1 360px", minWidth: 0 }}>
                    <span className="gname">{s.title}</span><br />
                    <span className="mute" style={{ fontSize: 13 }}>{LAYERS.find((l) => l.id === s.layer)?.title} · {plural(s.files.length, "file")} · {plural(lines, "line")} · <span className="mono">{[...new Set(s.files.map((f) => prShort(f.pr)))].join(" + ")}</span></span>
                    <ul className="facts" style={{ marginTop: 8 }}>{s.reasons.map((r) => <li key={r}>{r}</li>)}</ul>
                  </span>
                  <button className="btn sm" onClick={() => goStep(i)}>Take a look</button>
                  <button className={"vbtn fine" + (saved.answers[s.id] === "fine" ? " on" : "")} style={{ minHeight: 36, padding: "0 16px" }} aria-pressed={saved.answers[s.id] === "fine"}
                    onClick={() => setSaved((v) => ({ ...v, answers: { ...v.answers, [s.id]: v.answers[s.id] === "fine" ? undefined : "fine" } }))}><Ic name="check" />Looks fine</button>
                  {saved.answers[s.id] && saved.answers[s.id] !== "fine" && <span className={"mark " + saved.answers[s.id]}>{ANSWER_TEXT[saved.answers[s.id]!]}</span>}
                </div></Fragment>
              );
            })}
          </div></div>
        )}

        {screen === "files" && review && (() => {
          const prs = review.prs.map((p) => p.summary.pr);
          const curPr = prs.find((p) => prKey(p) === fileSel?.pr) ?? prs[0];
          const list = review.files.filter((f) => samePr(f.pr, curPr));
          const sel = list.find((f) => f.path === fileSel?.path) ?? list[0];
          // A big pull request has thousands of files: search them, and draw a few hundred at a time.
          const ff = fileFind.trim().toLowerCase();
          const found = ff ? list.filter((f) => f.path.toLowerCase().includes(ff)) : list;
          const shownFiles = found.slice(0, fileShow);
          const si = sel ? stepOfFile(sel) : -1;
          const skip = sel && review.skipped.find((s) => samePr(s.pr, sel.pr) && s.path === sel.path);
          const repoUrl = review.prs.find((p) => samePr(p.summary.pr, curPr))?.summary.url.replace(/\/pull\/\d+$/, "") ?? "";
          return (
            <div className={screenCls}><div className="wrap" style={{ maxWidth: 1360 }}>
              <div className="cols">
                <div className="aside side">
                  <div className="card" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 12 }}>
                    {prs.length > 1 && <div className="ptabs">{prs.map((p) => <button key={prKey(p)} className={"ptab" + (samePr(p, curPr) ? " on" : "")} onClick={() => setFileSel({ pr: prKey(p), path: "" })}>{prShort(p)}</button>)}</div>}
                    {list.length > 12 && <label className="search" style={{ minHeight: 36, flex: "none" }}><Ic name="search" /><input aria-label="Find a file" placeholder={`Find among ${num(list.length)} files`} value={fileFind} onChange={(e) => { setFileFind(e.target.value); setFileShow(300); }} /></label>}
                    {ff && <p className="mute" style={{ fontSize: 12 }}>{found.length ? plural(found.length, "file") + " match" : "No file matches."}</p>}
                    <div className="filelist">
                      {found.length > shownFiles.length && <button className="tag" style={{ order: 1, margin: "8px auto" }} onClick={() => setFileShow(fileShow + 300)}>Show {num(Math.min(300, found.length - shownFiles.length))} more of {num(found.length - shownFiles.length)}</button>}
                      {shownFiles.map((f) => {
                        const i = stepOfFile(f);
                        const name = f.path.slice(f.path.lastIndexOf("/") + 1);
                        return (
                          <button key={f.path} className={"frow" + (f === sel ? " on" : "")} onClick={() => setFileSel({ pr: prKey(curPr), path: f.path })}>
                            <span className="fn"><b>{name}</b><span>{f.path.slice(0, f.path.length - name.length)}</span></span>
                            <span className={"mark " + (i >= 0 ? stateOf(steps[i]) : "todo")}>{i < 0 ? "Skipped" : steps[i].tier === "glance" ? "Quick look" : `Decision ${i + 1}`}</span>
                          </button>
                        );
                      })}
                    </div>
                    {repoUrl && <Ext url={repoUrl} what="Browse the repository" host={curPr.host} />}
                  </div>
                </div>
                <div className="grow">
                  {sel && si >= 0 && (
                    <div className="card tint" style={{ display: "flex", flexWrap: "wrap", gap: "10px 16px", alignItems: "center", justifyContent: "space-between", padding: "14px 20px" }}>
                      <span style={{ flex: "1 1 280px" }}><span className="cap" style={{ color: "var(--acc)" }}>{steps[si].tier === "glance" ? "A quick look covers this file" : `Decision ${si + 1} asks about this file`}</span><br /><b>{steps[si].title}</b></span>
                      <button className="btn go sm" onClick={() => goStep(si)}>{steps[si].tier === "glance" ? "Open it" : "Go to the decision"}<Ic name="next" /></button>
                    </div>
                  )}
                  {sel && si < 0 && <div className="card" style={{ padding: "14px 20px" }}><span className="cap">No decision needed here</span><br />{skip ? `Skipped: ${skip.reason.toLowerCase()}.` : "Looksie skipped this file."} You can still read it.</div>}
                  {sel && (
                    <div className="diff scrollx">
                      <FileHead file={sel}>{diffSwitch}<Ext url={sel.url} what="Whole file" host={sel.pr.host} onCard /></FileHead>
                      <Diff key={prKey(sel.pr) + sel.path} file={sel} saved={saved} onComment={setComment} split={split} threads={review.threads.filter((t) => samePr(t.pr, sel.pr))} />
                    </div>
                  )}
                </div>
              </div>
            </div></div>
          );
        })()}

        {screen === "submit" && review && (
          <div className={screenCls}><div className="wrap">
            <div><h1 className="h1">Ready to send?</h1><p className="mute" style={{ marginTop: 6 }}>{answered} of {total} decisions answered. Each pull request gets its own verdict.</p></div>
            <div className="cols">
              <div className="grow" style={{ gap: 12 }}>
                {submissions.map((s) => {
                  const p = review.prs.find((x) => samePr(x.summary.pr, s.pr))!;
                  const own = !!auth && p.summary.author.toLowerCase() === auth.login.toLowerCase();
                  return (
                    <div key={prKey(s.pr)} style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                      <div className="prsec">
                        <span style={{ flex: "1 1 300px", minWidth: 0 }}><span className="gname">{p.summary.title}</span><br /><span className="mute mono">{prLabel(s.pr)}</span> <span className="mute" style={{ fontSize: 12 }}>· {plural(s.comments.length, "comment")}</span></span>
                        <span style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                          {(["approve", "request-changes", "comment"] as Verdict[]).map((v) => (
                            <button key={v} disabled={own && v !== "comment"} className={"rvb " + (v === "request-changes" ? "changes" : v) + (s.verdict === v ? " on" : "")} aria-pressed={s.verdict === v}
                              onClick={() => setSaved((x) => ({ ...x, verdicts: { ...x.verdicts, [prKey(s.pr)]: v } }))}>{VERDICT_TEXT[v]}</button>
                          ))}
                        </span>
                      </div>
                      {own && <p className="mute" style={{ fontSize: 12 }}>You opened this pull request, so {hostName(s.pr.host)} only accepts a comment from you.</p>}
                      {s.comments.length === 0 && <p className="mute">No comments for this one.</p>}
                      {s.comments.map((c, i) => (
                        <div className="dcard" key={i}><span className="mute mono">{c.anchor ? `${c.anchor.path}, line ${c.anchor.endLine}` : "On the pull request as a whole"}</span><p style={{ marginTop: 8 }}>{c.body}</p></div>
                      ))}
                    </div>
                  );
                })}
                {payloads && <div className="card"><div className="chd"><h2 className="cap">Exactly what would be sent</h2><button className="tag" onClick={() => setPayloads(null)}>Hide</button></div><pre className="payload">{payloads}</pre></div>}
              </div>
              <div className="aside side">
                <div className="card" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
                  {answered < total && <p><span className="flag look">{plural(total - answered, "decision")} not answered yet</span></p>}
                  <label className="cap">{review.prs.length > 1 ? "Summary (added to each review)" : "Summary"}
                    <textarea style={{ minHeight: 130, fontWeight: 400 }} value={saved.summary} onChange={(e) => setSaved((v) => ({ ...v, summary: e.target.value }))} />
                  </label>
                  <button className="btn" style={{ justifyContent: "center" }} onClick={async () => setPayloads(JSON.stringify(await Promise.all(submissions.map((s) => api.reviewPayload(s))), null, 2))}>Show what will be sent</button>
                  <button className="btn go" style={{ justifyContent: "center", minHeight: 52, fontSize: 16 }} disabled={sending || sendable.length === 0} onClick={submit}>{sending ? "Sending…" : `Submit ${plural(sendable.length, "review")}`}</button>
                  {submissions.some(postedHere) && <p className="mute" style={{ fontSize: 12 }}>Already posted: {submissions.filter(postedHere).map((s) => <a key={prKey(s.pr)} href={posted[prKey(s.pr)].url} onClick={(e) => { e.preventDefault(); api.openUrl(posted[prKey(s.pr)].url); }}>{prLabel(s.pr)} </a>)}</p>}
                  {sendable.length < submissions.length && <p className="mute" style={{ fontSize: 12 }}>{sendable.length === 0 ? "Nothing to send yet: add a summary, a comment, or an approval." : `${plural(submissions.length - sendable.length, "pull request")} with nothing to say will be left alone.`}</p>}
                  <p className="mute" style={{ fontSize: 12 }}>Posts to {hostName(review.prs[0].summary.pr.host)} as {auth?.login ?? "you"}, one review per pull request. Nothing is sent until you press Submit and confirm.</p>
                </div>
              </div>
            </div>
          </div></div>
        )}

        {screen === "done" && review && sent && (
          <div className={screenCls}><div className="wrap" style={{ minHeight: "100%", flexDirection: "row", flexWrap: "wrap", alignItems: "center", justifyContent: "center", gap: 56 }}>
            <div style={{ display: "flex", flexDirection: "column", gap: 24, alignItems: "center" }}>
              {(() => {
                const failed = sent.some((s) => s.error);
                const overall = sent.some((s) => s.verdict === "request-changes") ? "changes" : sent.every((s) => s.verdict === "approve") ? "approve" : "comment";
                return <div className={"stamp " + (failed ? "changes" : overall) + (answered < total ? " smudged" : "")}>{failed ? "Not all sent" : overall === "approve" ? "Approved" : overall === "changes" ? "Changes requested" : "Commented"}</div>;
              })()}
              {answered < total && <p className="mute">Stamped with decisions unanswered. It shows.</p>}
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {sent.map((s) => (
                  <p key={s.pr} style={{ display: "flex", gap: 10, justifyContent: "space-between", alignItems: "center", minWidth: 300 }}>
                    <span className="mono">{s.pr}</span>
                    {s.url ? <Ext url={s.url} what={VERDICT_TEXT[s.verdict]} host={review.prs[0].summary.pr.host} /> : <span className="mark change" title={s.error}>Failed: {s.error}</span>}
                  </p>
                ))}
              </div>
              <button className="btn" onClick={() => { go("inbox"); refresh(); }}><Ic name="back" />Back to the inbox</button>
            </div>
            <div className="receipt">
              <p className="serif" style={{ fontSize: 20, textAlign: "center" }}>Review receipt</p>
              <p className="r"><b>{review.briefing.title}</b></p>
              <p className="r mute"><span>{plural(review.files.reduce((a, f) => a + f.additions + f.deletions, 0), "line")} · {plural(review.files.length, "file")}</span><span>{plural(total, "decision")}</span></p>
              <p className="r"><span>Decisions</span><span className="rdots">{decisions.slice(0, 24).map((s) => <span key={s.id} className={"rdot " + (saved.answers[s.id] ?? "todo")} />)}</span></p>
              <p className="r"><span>{plural(submissions.reduce((a, s) => a + s.comments.length, 0), "comment")} posted</span><span /></p>
              <p className="serif" style={{ textAlign: "center", borderTop: "1px solid var(--line)", paddingTop: 12, marginTop: 6, fontSize: 17 }}>Had a proper looksie.</p>
            </div>
          </div></div>
        )}

        {update && updateNote !== "hidden" && (
          <div className="uptoast" role="status">
            <div style={{ display: "flex", gap: 12, alignItems: "flex-start" }}>
              <Logo size={34} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <b>Looksie {update.version} is ready</b>
                <p className="mute" style={{ fontSize: 13 }}>{updateNote === "installing" ? "Installing. Looksie will reopen in a moment." : "Restart when it suits you. Your answers and notes are kept."}</p>
              </div>
              <button className="x" aria-label="Later" title="Later" onClick={() => setUpdateNote("hidden")}><Ic name="cross" /></button>
            </div>
            {updateNote === "notes" && update.notes && <p className="note" style={{ whiteSpace: "pre-wrap", maxHeight: 180, overflow: "auto" }}>{update.notes}</p>}
            {updateNote !== "installing" && (
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                <button className="btn go sm" onClick={() => { setUpdateNote("installing"); update.install().catch(() => setUpdateNote(null)); }}>Restart and update</button>
                {update.notes && updateNote !== "notes" && <button className="btn sm" onClick={() => setUpdateNote("notes")}>What's new</button>}
              </div>
            )}
          </div>
        )}

        {about && (
          <>
            <button className="scrim" aria-label="Close" onClick={() => setAbout(false)} />
            <div className="sheet" role="dialog" aria-label="About Looksie">
              <div className="body" style={{ gap: 16 }}>
                <div style={{ display: "flex", gap: 14, alignItems: "center" }}><Logo size={56} /><div><h2 className="gname" style={{ fontSize: 24 }}>Looksie {version}</h2><p className="mute">Have a proper looksie at big pull requests.</p></div></div>
                <div className="card csec" style={{ boxShadow: "none", background: "var(--bg)" }}>
                  <div className="chd"><h2 className="cap">Updates</h2>{update && <span className="tag ok">{update.version} ready</span>}</div>
                  <p style={{ fontSize: 14 }}>Checked when Looksie opens and every few hours. They download quietly and install when you say.</p>
                  <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap", marginTop: 8 }}>
                    {update ? <button className="btn go sm" onClick={() => { setUpdateNote("installing"); update.install().catch(() => setUpdateNote(null)); }}>Restart and update</button> : <button className="btn sm" onClick={() => checkForUpdate(true)}><Ic name="refresh" />Check for updates</button>}
                    {checkState && !update && <span className="mute" style={{ fontSize: 13 }}>{checkState}</span>}
                    {checkState?.startsWith("Could not") && <button className="tag ext" onClick={() => api.openUrl("https://looksie.dev/")}>Download it from the website<Ic name="external" /></button>}
                  </div>
                </div>
                <div className="card csec" style={{ boxShadow: "none", background: "var(--bg)" }}>
                  <div className="chd"><h2 className="cap">What leaves this computer</h2></div>
                  <p style={{ fontSize: 14 }}>Requests to GitHub, as you. Prompts to your AI, if it is on (you can read them first). A daily update check. That is it.</p>
                </div>
                <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                  <button className="tag ext" onClick={() => api.openUrl("https://looksie.dev/")}>Website<Ic name="external" /></button>
                  <button className="tag ext" onClick={() => api.openUrl("https://looksie.dev/privacy.html")}>Privacy<Ic name="external" /></button>
                  <button className="tag ext" onClick={() => api.openUrl("https://looksie.dev/terms.html")}>Terms<Ic name="external" /></button>
                  <button className="tag ext" onClick={() => api.openUrl("https://github.com/marcus-smallman-lewis/looksie-releases/releases")}>Release notes<Ic name="external" /></button>
                </div>
              </div>
              <div className="foot"><button className="btn" style={{ justifyContent: "center" }} onClick={() => setAbout(false)}>Close</button></div>
            </div>
          </>
        )}

        {adding && <AddAccount host={adding.host} onClose={() => setAdding(null)} onAdded={(p) => { setAdding(null); return switchTo(p); }} />}

        {how && (
          <>
            <button className="scrim" aria-label="Close" onClick={() => setHow(false)} />
            <div className="sheet" role="dialog" aria-label="How Looksie decides">
              <div className="body" style={{ gap: 14 }}>
                <h2 className="serif" style={{ fontSize: 24 }}>How Looksie decides</h2>
                <p>Looksie splits a change into steps by layer (data, server, screens, tests and build, docs). Files in the same place go in one step.</p>
                <div><p className="cap" style={{ marginBottom: 6 }}>What makes a step a decision (they add up)</p>
                  <ul className="ticks"><li>a rule found something: a package dropped from the lockfile, a workflow that cannot run, a new secret or setting, stored data being changed, tests switched off, or folders that always change together where only one did</li><li>it touches sign-in, money or secrets</li><li>it changes code that already ran, rather than only adding new code</li><li>other steps build on it, or no changed test covers it</li><li>the author left a TODO or an assumption in it</li><li>it changes the database's shape, or how it builds and deploys</li></ul></div>
                <div><p className="cap" style={{ marginBottom: 6 }}>It is a quick look when</p>
                  <ul className="ticks"><li>none of that applies and it is small</li><li>it is only tests or only docs</li><li>it is the same edit repeated across many files. Any file that breaks the pattern is called out, and makes it a decision</li></ul></div>
                <p>Bigger reviews get more decisions, but slowly: about 3 for a small pull request, about 12 for a very big one. In a review of 1,000 lines or more, the places go to the steps with the most risk for the lines you would read. Past that, steps that scored like decisions wait first in the quick looks, marked as over the limit, and one click makes them all decisions. These are never held back by the limit: anything a rule flags, and changes to files whose path points to sign-in, money or secrets, to stored data, or to how it builds and deploys.</p>
                <p>Inside each step, the files most likely to need a fix for their size come first.</p>
                <p>Generated and vendored files are set aside, but are still in Files.</p>
                <p>With your AI on, it adds its own question to each decision and may flag a step too. Its flags are marked as AI.</p>
                <p className="note">This is a guide to where to look, not a verdict. Open any quick look and answer it like a decision if you want to. Each step shows the reasons that applied to it.</p>
              </div>
              <div className="foot"><button className="btn" style={{ justifyContent: "center" }} onClick={() => setHow(false)}>Close</button></div>
            </div>
          </>
        )}

        {promptText !== null && (
          <>
            <button className="scrim" aria-label="Close" onClick={() => setPromptText(null)} />
            <div className="sheet wide" role="dialog" aria-label="What will be sent">
              <div className="body">
                <h2 className="serif" style={{ fontSize: 24 }}>What will be sent</h2>
                <p className="mute" style={{ marginBottom: 8 }}>This is the whole prompt, {promptText.length.toLocaleString("en-US")} characters. With your API key it goes to {aiCompany?.name ?? "the AI company whose key you add"}, straight from this computer, and nowhere else. Your own agent gets the same text when it asks Looksie.</p>
                <pre className="payload" style={{ maxHeight: "none" }}>{promptText}</pre>
              </div>
              <div className="foot"><button className="btn" style={{ justifyContent: "center" }} onClick={() => setPromptText(null)}>Close</button></div>
            </div>
          </>
        )}

        {outline && review && (
          <>
            <button className="scrim" aria-label="Close" onClick={() => setOutline(false)} />
            <div className="sheet" role="dialog" aria-label="All steps">
              <div className="body">
                <h2 className="serif" style={{ fontSize: 24 }}>All steps</h2>
                <p className="mute" style={{ marginBottom: 8 }}>{answered} of {total} decisions answered, plus {plural(routine.length, "quick look")}. Jump anywhere; your answers stay put.</p>
                <div className="ptabs" style={{ marginBottom: 8 }}><button className={"ptab" + (flagFirst ? "" : " on")} onClick={() => setFlagFirst(false)}>In reading order</button><button className={"ptab" + (flagFirst ? " on" : "")} onClick={() => setFlagFirst(true)}>Flagged first</button></div>
                {(() => {
                  const row = (s: Step) => { const i = steps.indexOf(s); return (
                    <button key={s.id} className={"step" + (i === stepIdx ? " on" : "")} onClick={() => goStep(i)}>
                      <span className="mute" style={{ width: 24 }}>{s.tier === "glance" ? "·" : i + 1}</span><span style={{ flex: 1, minWidth: 0 }}><b style={{ fontWeight: 500 }}>{s.title}</b><br /><span className="mute mono">{[...new Set(s.files.map((f) => prShort(f.pr)))].join(" + ")}</span></span><span className={"mark " + stateOf(s)}>{markOf(s)}</span>
                    </button>); };
                  if (flagFirst) { const rank = { problem: 0, look: 1, todo: 2, question: 3, change: 3, fine: 4 } as Record<string, number>; return [...steps].sort((a, b) => rank[stateOf(a)] - rank[stateOf(b)]).map(row); }
                  return LAYERS.map((l) => { const xs = steps.filter((s) => s.layer === l.id); return xs.length ? <div key={l.id}><div className="chd" style={{ margin: "12px 0 2px", padding: "0 12px" }}><h2 className="cap">{l.title}</h2><span className="mute" style={{ fontSize: 12 }}>{plural(xs.length, "step")}</span></div>{xs.map(row)}</div> : null; });
                })()}
              </div>
              <div className="foot"><button className="btn" style={{ justifyContent: "center" }} onClick={() => setOutline(false)}>Close</button></div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function Links({ step, steps, stateOf, onPick, inline }: { step: Step; steps: Step[]; stateOf: (s: Step) => string; onPick: (i: number) => void; inline?: boolean }) {
  const chips = (ids: string[]) => ids.map((id) => steps.findIndex((s) => s.id === id)).filter((i) => i >= 0).map((i) => (
    <button key={i} className={"mark chip " + stateOf(steps[i])} title={`${steps[i].tier === "glance" ? "Quick look" : `Decision ${i + 1}`}: ${steps[i].title}`} onClick={() => onPick(i)}><span className="chip-k">{steps[i].tier === "glance" ? "Quick look" : `Decision ${i + 1}`} ·</span><span className="chip-t">{steps[i].title}</span></button>
  ));
  const on = chips(step.buildsOn), to = chips(step.leadsTo);
  const none = <span className="mute" style={{ fontSize: 13 }}>nothing else in this review</span>;
  if (inline) return (<><span className="cap">Builds on</span>{on.length ? on : none}<span className="cap" style={{ marginLeft: 12 }}>Leads to</span>{to.length ? to : none}</>);
  return (
    <>
      {on.length > 0 && <div><h2 className="cap" style={{ marginBottom: 6 }}>Builds on</h2><div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>{on}</div></div>}
      {to.length > 0 && <div><h2 className="cap" style={{ marginBottom: 6 }}>Leads to</h2><div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>{to}</div></div>}
    </>
  );
}

function AddAccount({ host, onClose, onAdded }: { host: string; onClose: () => void; onAdded: (p: Promise<Account[]>) => Promise<Account[]> }) {
  const [token, setToken] = useState("");
  const [server, setServer] = useState(host);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const clean = server.trim().replace(/^https?:\/\//, "").split("/")[0] || "github.com";
  return (
    <>
      <button className="scrim" aria-label="Close" onClick={onClose} />
      <form className="sheet" role="dialog" aria-label="Add a GitHub account" onSubmit={(e) => { e.preventDefault(); setBusy(true); setProblem(null); onAdded(api.addAccount(token, server)).catch((x) => setProblem(String(x.message ?? x))).finally(() => setBusy(false)); }}>
        <div className="body" style={{ gap: 16 }}>
          <div><h2 className="gname" style={{ fontSize: 22 }}>Add a GitHub account</h2><p className="mute" style={{ marginTop: 4 }}>For a second account, or a company GitHub server.</p></div>
          <div className="card tint" style={{ padding: "14px 16px", borderRadius: 16 }}><b>Easiest: the GitHub CLI</b><p style={{ fontSize: 13.5, marginTop: 4 }}>Run <span className="mono">gh auth login</span> in a terminal (add <span className="mono">--hostname</span> for a company server). Looksie finds the account by itself; press Look again in Settings.</p></div>
          <div className="formrow"><label htmlFor="tok">Or paste an access token</label><input id="tok" className="field sq" type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} placeholder="ghp_… or github_pat_…" />
            <span className="hint">It needs to read pull requests, and to write reviews if you want to send them from Looksie. <Ext url={`https://${clean}/settings/tokens/new?scopes=repo&description=Looksie`} what="Create one" host="github" /></span></div>
          <div className="formrow"><label htmlFor="srv">Server address <span className="mute" style={{ fontWeight: 400 }}>(only for a company GitHub)</span></label><input id="srv" className="field sq" value={server} onChange={(e) => setServer(e.target.value)} placeholder="github.com" /><span className="hint">Leave as github.com unless your company runs its own.</span></div>
          <p className="note">The token is checked with that server, then kept in this computer's keychain. It is never written to a file and never sent anywhere else.</p>
          {problem && <p style={{ color: "var(--bad)" }}>{problem}</p>}
        </div>
        <div className="foot"><button className="btn go" type="submit" style={{ justifyContent: "center" }} disabled={busy || !token.trim()}>{busy ? "Checking…" : "Check and add"}</button><button className="btn" type="button" style={{ justifyContent: "center" }} onClick={onClose}>Cancel</button></div>
      </form>
    </>
  );
}

/** An on/off switch with a label and a one-line hint. */
function Switch({ on, onChange, label, hint }: { on: boolean; onChange: (v: boolean) => void; label: string; hint?: string }) {
  return (
    <button className="setrow" role="switch" aria-checked={on} onClick={() => onChange(!on)}>
      <span className="txt"><b>{label}</b>{hint && <span className="mute">{hint}</span>}</span>
      <span className={"sw" + (on ? " on" : "")}><i /></span>
    </button>
  );
}

/** An Anthropic or OpenAI API key: paste it once; it is checked with that company (free) and kept in the system keychain. */
function AiKey({ company, onChange }: { company: AiProvider; onChange: (s: AiSetup) => void }) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  // A half-typed key or an error belongs to the company it was typed for.
  useEffect(() => { setKey(""); setProblem(null); }, [company.id]);
  const save = () => { setBusy(true); setProblem(null); api.setAiKey(company.id, key).then((s) => { setKey(""); onChange(s); }).catch((x) => setProblem(String(x.message ?? x))).finally(() => setBusy(false)); };
  if (company.hasKey) {
    return (
      <div className="arow" style={{ borderTop: 0 }}>
        <span className="dot ok" />
        <div className="who"><b>{company.name} key ending {company.keyHint}</b><span>Kept in this computer's keychain. Only ever sent to {company.name}.</span></div>
        <div className="acts"><button className="btn sm" onClick={() => api.removeAiKey(company.id).then(onChange).catch(() => undefined)}>Remove</button></div>
      </div>
    );
  }
  return (
    <form className="formrow" onSubmit={(e) => { e.preventDefault(); if (key.trim()) save(); }}>
      <input className="field sq" aria-label={`${company.name} API key`} type="password" autoComplete="off" spellCheck={false} placeholder={`${company.keyStart}…`} value={key} onChange={(e) => setKey(e.target.value)} />
      <span className="hint">Create one in <button type="button" className="linkish" onClick={() => api.openUrl(company.keysUrl)}>{company.keysPage}</button>. You pay {company.name} per review: usually a few cents.</span>
      {problem && <span style={{ color: "var(--bad)", fontSize: 13 }}>{problem}</span>}
      <span><button className="btn go sm" type="submit" disabled={busy || !key.trim()}>{busy ? "Checking…" : "Check and save"}</button></span>
    </form>
  );
}

/** Puts text on the clipboard. The modern way first; if the window says no, the old way, which still works in the app. */
async function copyText(text: string): Promise<boolean> {
  try {
    await api.copyText(text);
    return true;
  } catch {
    const t = document.createElement("textarea");
    t.value = text;
    t.setAttribute("readonly", "");
    t.style.position = "fixed";
    t.style.opacity = "0";
    document.body.appendChild(t);
    t.select();
    const ok = document.execCommand("copy");
    t.remove();
    return ok;
  }
}

/** A line to paste somewhere else, with a Copy button. */
/** One agent in Settings: connect it in one click, or say why not. */
function AgentRow({ agent: a, onChange }: { agent: AgentState; onChange: (s: AgentState[]) => void }) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const live = a.connected && !a.elsewhere;
  const act = (connect: boolean) => {
    setBusy(true);
    setErr(null);
    (connect ? api.connectAgent(a.id) : api.disconnectAgent(a.id)).then(onChange).catch((e) => setErr(String(e))).finally(() => setBusy(false));
  };
  return (
    <div className="agentrow">
      <div className="t">
        <b>{live && <span className="dot ok" />}{a.name}</b>
        <span className="mute">
          {busy ? (live ? "Disconnecting…" : "Connecting…")
            : live ? <>Connected. In a new {a.name} session, type <span className="mono">{a.command}</span> to review the pull request open here, or just ask it to review a pull request.</>
            : a.elsewhere ? "Connected to another copy of Looksie. Connect again to use this one."
            : a.found ? "Not connected yet."
            : "Not found on this computer."}
        </span>
        {err && <span style={{ color: "var(--bad)", fontSize: 12.5 }}>{err}</span>}
      </div>
      {busy ? <span className="spin sm" />
        : live ? <button className="btn sm" onClick={() => act(false)}>Disconnect</button>
        : <button className="btn go sm" disabled={!a.found} onClick={() => act(true)}>Connect</button>}
    </div>
  );
}

function Copyable({ label, text, onCopy }: { label: string; text: string; onCopy?: () => void }) {
  const [done, setDone] = useState(false);
  return (
    <div className="copyable">
      <span className="cap">{label}</span>
      <div className="row">
        <code className="mono">{text}</code>
        <button className="btn sm" onClick={() => { copyText(text).then((ok) => { if (!ok) return; setDone(true); onCopy?.(); window.setTimeout(() => setDone(false), 1800); }); }}>{done ? "Copied" : "Copy"}</button>
      </div>
    </div>
  );
}

function PlanActions({ shop, onChange }: { shop: Shop; onChange: (s: Shop) => void }) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const run = (p: Promise<Shop>) => { setBusy(true); setProblem(null); p.then(onChange).catch((e) => setProblem(String(e.message ?? e))).finally(() => setBusy(false)); };
  const buy = (url: string, label: string) => (
    <button className="btn" disabled={!url} title={url ? "Opens the checkout in your browser" : "The checkout is being set up"} onClick={() => api.openUrl(url)}>{label}<Ic name="external" /></button>
  );
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      {!shop.status.trialUsed && <button className="btn go" style={{ justifyContent: "center", minHeight: 52, fontSize: 16 }} disabled={busy} onClick={() => run(api.startTrial())}>Start the {shop.trialDays}-day free trial</button>}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>{buy(shop.checkoutYearly, `Buy yearly · ${shop.priceYearly}`)}{buy(shop.checkoutMonthly, `Buy monthly · ${shop.priceMonthly}`)}</div>
      {!shop.checkoutYearly && <p className="mute" style={{ fontSize: 12 }}>The checkout is being set up. The buttons will open it as soon as it is live.</p>}
      <label className="cap" htmlFor="lic">Already bought? Paste your licence key
        <input id="lic" className="field" style={{ display: "block", width: "100%", marginTop: 6, fontWeight: 400 }} placeholder="It activates as soon as you paste it" value={key} disabled={busy}
          onChange={(e) => setKey(e.target.value)}
          onPaste={(e) => { const t = e.clipboardData.getData("text"); if (t.trim().length >= 16) { e.preventDefault(); setKey(t.trim()); run(api.activateLicence(t)); } }}
          onKeyDown={(e) => { if (e.key === "Enter" && key.trim()) run(api.activateLicence(key)); }} />
      </label>
      {problem && <p style={{ color: "var(--bad)" }}>{problem}</p>}
    </div>
  );
}
