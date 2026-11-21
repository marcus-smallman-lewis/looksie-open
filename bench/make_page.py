"""Writes site/benchmarks.html from the benchmark's result files, so every number on the page is the measured one.

    python bench/make_page.py

Reads (from bench/out): results_at_v12_all.json (this version, every pull request), results_at_v12_test.json (this
version, held-out repositories only) and results_at_v11_all.json (the previous version). All made with BENCH_AT=1:
each pull request as it was when its reviewer first commented.
"""
import io, json, os, datetime

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SITE = os.path.join(ROOT, 'site')
OUT = os.path.join(HERE, 'out')
load = lambda name: json.load(open(os.path.join(OUT, name), encoding='utf-8'))
ALLR, TEST, V11 = load('results_at_v12_all.json'), load('results_at_v12_test.json'), {s['label']: s for s in load('results_at_v11_all.json')['summary']}
sums = {s['label']: s for s in ALLR['summary']}
ALL = sums['All']
BIG = sums.get('size: over 5,000 lines')
HELD = TEST['summary'][0]
rows = ALLR['rows']
VERSION = json.load(open(os.path.join(ROOT, 'app', 'src-tauri', 'tauri.conf.json'), encoding='utf-8'))['version']
repos = sorted({r['repo'] for r in rows})
langs = sorted({r['language'] for r in rows})
agents = sum(1 for r in rows if r['kind'] == 'agent')
where = ALL['where']


def pct(x):
    return f'{round(100 * x)}%'


def bar(label, value, cls=''):
    return f'<div class="bar {cls}"><span class="bl">{label}</span><span class="track"><span class="fill" style="width:{100 * value:.1f}%"></span></span><span class="bv">{pct(value)}</span></div>'


def card(title, note, bars, foot):
    return f'<div class="card"><h3>{title}</h3><p class="mute">{note}</p>{"".join(bars)}<p class="mute small">{foot}</p></div>'


def recall_card(s, title, note):
    return card(title, note, [bar('Looksie decisions', s['recall'], 'me'), bar('Random files, same number of lines', s['recall_random']), bar('Biggest files first, same number of lines', s['recall_biggest'])],
                f"{s['prs']} pull requests · {s['fixed_files']} files that needed fixing · the decisions cover {pct(s['lines_read_share'])} of the changed lines · {s['decisions_median']} decisions (median)")


def change_card():
    old, new = V11['size: over 5,000 lines'], BIG
    return card('This version: same fixes, less reading on big pull requests',
                "In reviews of 1,000 lines or more, decisions are now chosen by risk for the lines you read, rather than by the highest score, which favoured big steps. Pull requests over 5,000 lines:",
                [bar(f'Looksie {VERSION}: share of the change in decisions', new['lines_read_share'], 'me'), bar('Looksie before this version', old['lines_read_share'])],
                f"The decisions held {new['where']['decision']} of the {new['fixed_files']} files reviewers asked to have fixed, against {old['where']['decision']} before. Smaller pull requests are unchanged. On the repositories this rule was not developed on, it made no measurable difference; they had few pull requests this big.")


def order_card(s, title, note, before=None):
    bars = [bar(f'Looksie {VERSION} order', s['found_after']['20%'], 'me')]
    if before:
        bars.append(bar('Looksie before this version', before['found_after']['20%']))
    bars.append(bar('Random order', s['found_after_random']['20%']))
    return card(title, note, bars, f"Share of the files that needed fixing met after reading the first 20% of each pull request's changed lines. {s['prs']} pull requests.")


table = ''.join(
    f"<tr><td>{s['label'].split(': ', 1)[-1]}</td><td>{s['prs']}</td><td>{s['fixed_files']}</td><td><b>{pct(s['recall'])}</b></td><td>{pct(s['recall_random'])}</td><td>{pct(s['lines_read_share'])}</td><td>{pct(s['found_after']['20%'])}</td></tr>"
    for k, s in sums.items() if k.startswith(('language:', 'size:', 'kind:')))

head = io.open(os.path.join(SITE, 'privacy.html'), encoding='utf-8').read()
head = head[:head.index('<main class="doc">')]
head = head.replace('<title>Privacy · Looksie</title>', '<title>Looksie benchmarks</title>')
head = head.replace('<meta name="description" content="What Looksie keeps on your computer, and the little that leaves it.">',
                    '<meta name="description" content="How often the files real reviewers asked to have fixed end up in Looksie\'s decisions, on real open-source pull requests.">')
head = head.replace('@media (max-width:640px)', '''.doc h3{font-size:18px;font-weight:700;margin:0 0 4px}
.doc .card{border:1px solid var(--line);border-radius:18px;padding:18px 20px;margin-top:16px}
.doc .mute{color:var(--mute)}.doc .small{font-size:14px;margin-top:10px}
.bar{display:grid;grid-template-columns:minmax(150px,240px) 1fr 48px;gap:10px;align-items:center;margin-top:8px;font-size:15px}
.bar .track{height:12px;background:var(--bg);border-radius:999px;overflow:hidden}
.bar .fill{display:block;height:100%;background:var(--line);border-radius:999px}
.bar.me .fill{background:var(--acc)}.bar.me .bv,.bar.me .bl{font-weight:700}
.bar .bv{text-align:right;font-variant-numeric:tabular-nums}
.doc table{width:100%;border-collapse:collapse;margin-top:12px;font-size:15px}
.doc th,.doc td{text-align:left;padding:7px 8px;border-bottom:1px solid var(--line)}
.doc th{color:var(--mute);font-weight:500}.doc td:not(:first-child),.doc th:not(:first-child){text-align:right}
.tablewrap{overflow-x:auto}
@media (max-width:640px){.bar{grid-template-columns:1fr 44px}.bar .track{grid-column:1/-1;grid-row:2}}
@media (max-width:640px)''', 1)
today = datetime.date.today()
date = f'{today.day} {today.strftime("%B %Y")}'
big = recall_card(BIG, 'Big pull requests (over 5,000 lines)', 'What Looksie is built for. A decision is still a whole folder, so the next step is smaller decisions inside big folders, and grouping folders that got the same change.') if BIG and BIG['prs'] >= 5 else ''

body = f'''<main class="doc">
<h1>Benchmarks</h1>
<p class="meta">Looksie {VERSION}, rules only (no AI). Measured {date} on {ALL['prs']} merged pull requests from {len(repos)} open-source repositories in {len(langs)} languages.</p>
<div class="short"><b>The short version</b>
<ul>
<li>We took real pull requests where a reviewer asked for something to be fixed and the author then changed that code, and ran Looksie on each pull request <b>as it was when the reviewer first commented</b>, before any fix.</li>
<li>Looksie's decisions held <b>{pct(ALL['recall'])}</b> of the files that needed fixing. Reading the same amount in random order finds {pct(ALL['recall_random'])}; biggest files first, {pct(ALL['recall_biggest'])}.</li>
<li>Reading in Looksie's order, the first 20% of a pull request already reaches <b>{pct(ALL['found_after']['20%'])}</b> of them, against {pct(ALL['found_after_random']['20%'])} in random order.</li>
<li>On pull requests over 5,000 lines, this version's decisions cover {pct(BIG['lines_read_share'])} of the change, down from {pct(V11['size: over 5,000 lines']['lines_read_share'])}, for the same share of what needed fixing.</li>
</ul></div>

<h2>Results</h2>
{recall_card(ALL, 'All pull requests', "Files that reviewers asked to have fixed, and that were in one of Looksie's decisions.")}
{order_card(ALL, 'What you meet first', 'Reading the decisions in the order Looksie shows them, then the rest.')}
{change_card()}
{big}
{recall_card(HELD, 'Repositories the tuning never saw', 'The weights Looksie uses to weigh files were fitted on 21 of the 34 repositories. These are the other 13, which the weights never saw.')}

<h2>By language, size and author</h2>
<div class="tablewrap"><table>
<tr><th></th><th>PRs</th><th>Fixed files</th><th>In decisions</th><th>Random</th><th>Lines in decisions</th><th>First 20%</th></tr>
{table}
</table></div>
<p class="small mute">Small groups (under 10 pull requests) swing a lot. Read them as a hint, not a result.</p>

<h2>Where the misses were</h2>
<p>Of {sum(where.values())} files that needed fixing, {where['decision']} were in a decision. Of the rest, {where['quick-look']} were quick looks, {where['over-limit']} were over the decision limit, and {where['skipped']} were set aside as generated. Most quick-look misses are test-only and docs-only steps: reviewers often ask for tests to be fixed. We tried making those decisions too, and it meant reading most of every pull request, so we have not.</p>

<h2>How it was measured</h2>
<ul>
<li><b>The pull requests:</b> recently merged, 4 or more files and 150 or more changed lines, with at least one review comment that led to a change, from {', '.join(repos)}. Many are over 2,000 lines. {agents} of them were written by a coding agent.</li>
<li><b>What needed fixing:</b> an inline review comment from a person (not a bot, and not the author) that GitHub later marked as outdated, meaning the code under it changed afterwards. Counted per file.</li>
<li><b>When:</b> each pull request as it was at the commit its reviewer first commented on, so the fixes are not in what Looksie sees.</li>
<li><b>Looksie:</b> the same engine as the app, from the command line, with no AI. A file counts if it is in a decision.</li>
<li><b>Baselines:</b> for each pull request, files read in random order (averaged over 500 shuffles) or biggest first, stopping before they read more lines than Looksie's decisions cover.</li>
<li><b>Held out:</b> the weights that weigh files were fitted on 21 of the 34 repositories and never on the other 13. This version's change to how decisions are chosen was developed on the 21; on the other 13 it made no measurable difference.</li>
</ul>

<h2>What this does not show</h2>
<ul>
<li>A comment can go out of date for other reasons, and reviewers do not comment on everything that deserves it. The answer key is a good signal, not the truth.</li>
<li>Only pull requests with at least one such comment are included, which leans towards pull requests where reviewers engaged.</li>
<li>It measures where to look, not whether a reviewer would spot the problem once there.</li>
<li>Few agent-written pull requests in big open-source repositories have human review comments yet, so the agent results rest on {agents} pull requests.</li>
<li>For an earlier commit, GitHub lists at most 300 changed files, and CI results are from the final commit.</li>
</ul>
<p>The scripts that collect the pull requests and score them, and the list of pull requests, are in the <a href="https://github.com/marcus-smallman-lewis/looksie-open/tree/main/bench">open-source repository</a>. Found a pull request where Looksie got it wrong? <a href="mailto:support@looksie.dev">Send it to us</a>. See also <a href="how.html">how Looksie decides</a>.</p>

</main>
<footer><a href="index.html">Looksie</a> · <a href="how.html">How it decides</a> · <a href="benchmarks.html">Benchmarks</a> · <a href="privacy.html">Privacy policy</a> · <a href="terms.html">Terms</a> · <a href="https://github.com/marcus-smallman-lewis/looksie-open">Source code</a></footer>
</div>
</body>
</html>
'''
io.open(os.path.join(SITE, 'benchmarks.html'), 'w', encoding='utf-8', newline='\n').write(head + body)
print('written', ALL['prs'], 'PRs;', HELD['prs'], 'held out')
