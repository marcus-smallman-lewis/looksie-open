// Small shared pieces: icons, the logo, the "opens outside Looksie" link, and the diff view.

import { useId, useState, type ReactNode } from "react";
import { api } from "./api";
import { hostName, prKey, prShort, type FileChange, type Saved, type Thread } from "./types";

const ICONS: Record<string, string[]> = {
  inbox: ["M4 13l2.5-7h11L20 13v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1z", "M4 13h5a3 3 0 0 0 6 0h5"],
  back: ["M19 12H5M11 6l-5 6 5 6"],
  next: ["M5 12h14M13 6l6 6-6 6"],
  check: ["M5 12.5l4.5 4.5L19 7.5"],
  cross: ["M6 6l12 12M18 6L6 18"],
  ask: ["M9 9a3 3 0 1 1 4.5 2.6c-.9.5-1.5 1.2-1.5 2.4", "M12 18v.01"],
  plus: ["M12 5v14M5 12h14"],
  spark: ["M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"],
  moon: ["M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"],
  external: ["M14 5h5v5", "M19 5l-8 8", "M17 14v4a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1h4"],
  refresh: ["M20 11a8 8 0 1 0-2.3 5.7", "M20 5v6h-6"],
  badge: ["M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9z"],
  gear: ["M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z", "M18.4 13a7.5 7.5 0 0 0 0-2l2-1.5-2-3.4-2.3.9a7.5 7.5 0 0 0-1.7-1L14 3.6h-4L9.6 6a7.5 7.5 0 0 0-1.7 1l-2.3-.9-2 3.4 2 1.5a7.5 7.5 0 0 0 0 2l-2 1.5 2 3.4 2.3-.9a7.5 7.5 0 0 0 1.7 1l.4 2.4h4l.4-2.4a7.5 7.5 0 0 0 1.7-1l2.3.9 2-3.4z"],
  plug: ["M9 3v5M15 3v5", "M6 8h12v3a6 6 0 0 1-12 0z", "M12 17v4"],
  search: ["M11 4.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13z", "M16 16l4.5 4.5"],
  down: ["M6 9l6 6 6-6"],
  rows: ["M4 6h16M4 10h16M4 14h16M4 18h16"],
  cards: ["M6 4.5h12a2 2 0 0 1 2 2v2a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-2a2 2 0 0 1 2-2z", "M6 13.5h12a2 2 0 0 1 2 2v2a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-2a2 2 0 0 1 2-2z"],
  grid: ["M4 4h7v7H4z", "M13 4h7v7h-7z", "M4 13h7v7H4z", "M13 13h7v7h-7z"],
  info: ["M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17z", "M12 11v5", "M12 7.8v.01"],
};

export function Ic({ name }: { name: keyof typeof ICONS | string }) {
  return (
    <svg className="ic" viewBox="0 0 24 24" aria-hidden="true">
      {(ICONS[name] ?? []).map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  );
}

export function Logo({ looking, size }: { looking?: boolean; size?: number }) {
  // One rounded square clips everything, so each corner is drawn once (a band with its own rounded corners left a blurry edge).
  const clip = "lg-clip" + useId().replace(/:/g, "");
  return (
    <svg className={"logo" + (looking ? " looking" : "")} style={size ? { width: size, height: size } : undefined} viewBox="0 0 48 48" role="img" aria-label="Looksie">
      <defs><clipPath id={clip}><rect width="48" height="48" rx="13" /></clipPath></defs>
      <g clipPath={`url(#${clip})`}>
      <rect className="lg-bg" width="48" height="48" />
      <circle className="lg-eye" cx="15.2" cy="25" r="9" />
      <circle className="lg-eye" cx="32.8" cy="25" r="9" />
      <g className="lg-look">
        <circle className="lg-pupil" cx="17.2" cy="24.6" r="4.4" />
        <circle className="lg-pupil" cx="34.8" cy="24.6" r="4.4" />
        <circle className="lg-glint" cx="18.8" cy="22.9" r="1.4" />
        <circle className="lg-glint" cx="36.4" cy="22.9" r="1.4" />
      </g>
      <rect className="lg-band" y="29.5" width="48" height="18.5" />
      <rect className="lg-line" x="9" y="35" width="18" height="2.6" rx="1.3" />
      <rect className="lg-line" x="9" y="40.2" width="11" height="2.6" rx="1.3" />
      </g>
    </svg>
  );
}

/** A link that leaves Looksie. Always the same shape and icon, and named after the host it opens. */
export function Ext({ url, what, host, onCard }: { url: string; what: string; host: string; onCard?: boolean }) {
  const name = hostName(host);
  return (
    <button className={"tag ext" + (onCard ? " on-card" : "")} title={`Opens ${name} in your browser`} onClick={() => api.openUrl(url)}>
      {what} on {name}
      <Ic name="external" />
    </button>
  );
}

/** Shows a pull request description readably: headings, bullets and `code`, without a Markdown library. */
export function Prose({ text }: { text: string }) {
  const inline = (s: string, key: number) => (
    <span key={key}>
      {s.split(/(`[^`]+`|\*\*[^*]+\*\*)/g).map((part, i) =>
        part.startsWith("`") && part.endsWith("`") ? <code key={i} className="mono">{part.slice(1, -1)}</code> : part.startsWith("**") && part.endsWith("**") ? <b key={i}>{part.slice(2, -2)}</b> : part,
      )}
    </span>
  );
  return (
    <div className="prose">
      {text.split("\n").map((line, i) => {
        const t = line.trim();
        if (!t) return <div key={i} style={{ height: 8 }} />;
        const h = /^#{1,6}\s+(.*)$/.exec(t);
        if (h) return <p key={i} className="cap" style={{ marginTop: 6 }}>{h[1]}</p>;
        const b = /^(\s*)[-*]\s+(.*)$/.exec(line);
        if (b) return <p key={i} className="bullet" style={{ marginLeft: Math.min(b[1].length, 6) * 6 }}>{inline(b[2], i)}</p>;
        return <p key={i}>{inline(t, i)}</p>;
      })}
    </div>
  );
}

/** A description that folds away after a few lines when it is long. */
export function Folded({ text }: { text: string }) {
  const long = text.length > 520 || text.split(/\r?\n/).length > 8;
  const [open, setOpen] = useState(false);
  return (
    <div>
      <div className={long && !open ? "descfold" : undefined}><Prose text={text} /></div>
      {long && <button className="tag" style={{ marginTop: 8 }} onClick={() => setOpen(!open)}>{open ? "Show less" : "Show the whole description"}</button>}
    </div>
  );
}

const LIMIT = 400;

/** One file's changes, with comments. `focus` narrows a huge file to the lines that mention those words.
 *  `split` shows the old file on the left and the new one on the right; comments work the same either way. */
export function Diff({ file, focus, saved, onComment, threads, split }: { file: FileChange; focus?: string[] | null; saved: Saved; onComment: (key: string, text: string) => void; threads: Thread[]; split?: boolean }) {
  const [composing, setComposing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [all, setAll] = useState(false);
  // Which line's comments are open. Comments float over the diff by their line; closed, they are a marker in the margin.
  const [openKey, setOpenKey] = useState<string | null>(null);

  if (!file.hunks.length) {
    return <p className="mute" style={{ padding: "14px 22px" }}>{file.trimmed ? `Generated or vendored, so not shown here (${file.trimmed.toLocaleString("en-US")} changed lines). It is on ${hostName(file.pr.host)}.` : file.patch === null ? `${hostName(file.pr.host)} did not send this file's changes (too large, or not text).` : "Nothing to show."}</p>;
  }
  const cut = file.trimmed ? <p className="mute" style={{ padding: "10px 22px", fontSize: 13 }}>This file has {file.trimmed.toLocaleString("en-US")} changed lines; the rest are on {hostName(file.pr.host)}.</p> : null;

  type Row = { key: string; kind: string; no: number | null; oldNo: number | null; sign: string; text: string; head?: string };
  const rows: Row[] = [];
  for (const h of file.hunks) {
    let lines = h.lines.map((l, i) => ({ l, i }));
    if (focus?.length) {
      const hit = new Set<number>();
      h.lines.forEach((l, i) => {
        if (focus.some((f) => l.text.includes(f))) for (let k = i - 2; k <= i + 5; k++) hit.add(k);
      });
      lines = lines.filter((x) => hit.has(x.i));
      if (!lines.length) continue;
    }
    rows.push({ key: "h" + h.header + rows.length, kind: "hh", no: null, oldNo: null, sign: "", text: "", head: h.header });
    for (const { l } of lines) {
      const side = l.kind === "del" ? "old" : "new";
      const no = l.kind === "del" ? l.oldNo : l.newNo;
      rows.push({ key: `${prKey(file.pr)}|${file.path}|${side}|${no}`, kind: l.kind, no, oldNo: l.oldNo, sign: l.kind === "add" ? "+" : l.kind === "del" ? "-" : "", text: l.text });
    }
  }
  // Side by side: unchanged lines sit on both sides; a run of removed lines pairs up with the added lines after it.
  // A file that is only new (or only removed) has one side, so it stays in one column.
  const oneSided = rows.every((r) => r.kind === "hh" || r.kind === "add") || rows.every((r) => r.kind === "hh" || r.kind === "del");
  const sbs = !!split && !oneSided;
  type Pair = { key: string; head?: string; left?: Row; right?: Row; same?: boolean };
  const pairs: Pair[] = [];
  if (sbs) {
    for (let i = 0; i < rows.length; ) {
      const r = rows[i];
      if (r.kind === "hh") { pairs.push({ key: r.key, head: r.head }); i++; continue; }
      if (r.kind !== "del" && r.kind !== "add") { pairs.push({ key: r.key, left: r, right: r, same: true }); i++; continue; }
      const dels: Row[] = [], adds: Row[] = [];
      while (i < rows.length && rows[i].kind === "del") dels.push(rows[i++]);
      while (i < rows.length && rows[i].kind === "add") adds.push(rows[i++]);
      for (let k = 0; k < Math.max(dels.length, adds.length); k++) pairs.push({ key: (dels[k]?.key ?? "") + "~" + (adds[k]?.key ?? ""), left: dels[k], right: adds[k] });
    }
  }
  const total = sbs ? pairs.length : rows.length;
  const shownRows = all ? rows : rows.slice(0, LIMIT);
  const shownPairs = all ? pairs : pairs.slice(0, LIMIT);
  const shownCount = sbs ? shownPairs.length : shownRows.length;

  const close = () => { setComposing(null); setOpenKey(null); };
  const write = (key: string) => { setComposing(key); setOpenKey(key); setDraft(saved.comments[key] ?? ""); };

  /** One line with its comment marker, and its comments when open. `copy` is the old-side copy of an unchanged line:
   *  it has no comments of its own, they belong to the new side. */
  const line = (r: Row, idx: number, copy = false) => {
    const theirs = !copy && r.kind !== "del" ? threads.filter((t) => t.path === file.path && t.line === r.no) : [];
    const mine = copy ? undefined : saved.comments[r.key];
    const count = theirs.length + (mine ? 1 : 0);
    const isOpen = !copy && (openKey === r.key || composing === r.key);
    // Near the end of the file the box opens upwards, so it is never cut off.
    const up = idx > 6 && shownCount - idx <= 8;
    return (
      <div key={r.key + (copy ? "~old" : "")} className="lnwrap">
        <div className={"ln " + r.kind + (count ? " has-c" : "")}>
          {copy ? (
            <span className="lgap" />
          ) : count > 0 ? (
            <button className={"cmark" + (mine ? "" : " theirs")} aria-expanded={isOpen} aria-label={`${isOpen ? "Hide" : "Show"} ${count} comment${count === 1 ? "" : "s"} on line ${r.no}`} title={isOpen ? "Hide comments" : "Show comments"} onClick={() => (isOpen ? close() : setOpenKey(r.key))}>{count}</button>
          ) : (
            <button className="plus" aria-label={`Comment on line ${r.no}`} onClick={() => write(r.key)}><Ic name="plus" /></button>
          )}
          <span className="num">{copy ? r.oldNo : r.no}</span>
          <span className="sign">{r.sign}</span>
          <code>{r.text}</code>
        </div>
        {isOpen && (
          <div className={"cpop" + (up ? " up" : "")} role="dialog" aria-label={`Comments on line ${r.no}`} onKeyDown={(e) => { if (e.key === "Escape") close(); }}>
            <div className="cpop-hd"><span className="cap">Line {r.no}</span><button className="x" aria-label="Close" onClick={close}><Ic name="cross" /></button></div>
            {theirs.map((t) => (
              <div key={t.url} className="thread">
                <div className="who"><b>{t.author}</b><span className="tag on-card">Already on {hostName(file.pr.host)}</span></div>
                <p>{t.body}</p>
              </div>
            ))}
            {mine && composing !== r.key && (
              <div className="thread mine">
                <div className="who"><b>You</b><span className="tag on-card">Draft · not posted</span></div>
                <p>{mine}</p>
                <div className="acts"><button className="tag on-card" onClick={() => write(r.key)}>Edit</button><button className="tag on-card" onClick={() => { onComment(r.key, ""); close(); }}>Delete</button></div>
              </div>
            )}
            {composing === r.key ? (
              <div className="thread mine">
                <label>
                  {mine ? "Edit your comment" : "Comment on this line"}
                  <textarea autoFocus style={{ background: "var(--card)" }} value={draft} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => { if ((e.ctrlKey || e.metaKey) && e.key === "Enter" && draft.trim()) { onComment(r.key, draft.trim()); close(); } }} />
                </label>
                <div className="acts">
                  <button className="btn go sm" disabled={!draft.trim()} onClick={() => { onComment(r.key, draft.trim()); close(); }}>Save draft</button>
                  <button className="btn sm" onClick={() => (mine || theirs.length ? setComposing(null) : close())}>Cancel</button>
                </div>
              </div>
            ) : !mine && <button className="tag" style={{ alignSelf: "flex-start" }} onClick={() => write(r.key)}><Ic name="plus" />Add your comment</button>}
          </div>
        )}
      </div>
    );
  };
  const blank = <div className="lnwrap"><div className="ln none" /></div>;

  return (
    <div className={sbs ? "split" : undefined} style={{ minHeight: composing || openKey ? 300 : undefined }}>
      {sbs
        ? shownPairs.map((pr, idx) => pr.head !== undefined
          ? <div key={pr.key} className="hh">{pr.head}</div>
          : <div key={pr.key} className="lnpair">{pr.left ? line(pr.left, idx, pr.same) : blank}{pr.right ? line(pr.right, idx) : blank}</div>)
        : shownRows.map((r, idx) => (r.kind === "hh" ? <div key={r.key} className="hh">{r.head}</div> : line(r, idx)))}
      {total > LIMIT && !all && (
        <p style={{ padding: "12px 22px" }}><button className="btn sm" onClick={() => setAll(true)}>Show all {total.toLocaleString("en-US")} lines</button></p>
      )}
      {cut}
    </div>
  );
}

export function FileHead({ file, children, lead }: { file: FileChange; children?: ReactNode; lead?: ReactNode }) {
  return (
    <div className="fhead">
      {lead}
      <span className="tag on-card mono">{prShort(file.pr)}</span>
      <span>{file.path}</span>
      <span className="mute">+{file.additions} −{file.deletions}</span>
      <span className="sp" />
      {children}
    </div>
  );
}
