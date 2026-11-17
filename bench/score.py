"""Runs Looksie (rules only, no AI) on every pull request in bench/out/sample.json and scores it against the answer key.

    python bench/score.py              writes bench/out/results.json and prints a summary

The question: when a reviewer reads only Looksie's decisions, how many of the files that real reviewers asked to have
fixed did they read, and how much of the pull request did that take? Compared with two baselines given the same
amount of reading per pull request: the biggest files first, and files in random order.
Needs the command-line tool built first:  cargo build --release -p looksie-cli
"""
import json, os, random, subprocess, sys, time
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
OUT = os.path.join(HERE, 'out')
CACHE = os.path.join(OUT, 'reviews')
# Every answer GitHub gave, kept so the engine can be re-run offline (looksie-cli --cache).
HTTP = os.path.join(OUT, 'github_at' if os.environ.get('BENCH_AT') == '1' else 'github')
OFFLINE = os.environ.get('BENCH_OFFLINE') == '1'
FRESH = os.environ.get('BENCH_FRESH') == '1'
TAG = os.environ.get('BENCH_TAG', '')
EXE = os.environ.get('BENCH_EXE') or os.path.join(ROOT, 'target', 'release', 'looksie-cli' + ('.exe' if os.name == 'nt' else ''))
# BENCH_AT=1: review each pull request as it was at the commit its reviewer first commented on (bench/attach_commits.py),
# so the fixes the reviewers asked for are not in what Looksie sees.
AT = os.environ.get('BENCH_AT') == '1'
RANDOM_RUNS = 500


def review(repo, number, at=None):
    path = os.path.join(CACHE + TAG, f"{repo.replace('/', '_')}_{number}.json")
    if os.path.exists(path) and not FRESH:
        return json.load(open(path, encoding='utf-8'))
    for attempt in range(4):
        r = subprocess.run([EXE, '--cache', HTTP] + (['--offline'] if OFFLINE else []) + (['--at', at] if at else []) + ['review', f'{repo}#{number}', '--json'], capture_output=True, text=True, encoding='utf-8')
        if r.returncode == 0 or 'rate limit' not in r.stderr.lower():
            break
        # GitHub's hourly limit: wait for it, then carry on. Answers so far are cached, so nothing is asked twice.
        reset = subprocess.run(['gh', 'api', 'rate_limit', '-q', '.resources.core.reset'], capture_output=True, text=True).stdout.strip()
        wait = max(60, int(reset or 0) - int(time.time()) + 30) if reset.isdigit() else 900
        print(f'  waiting {wait // 60} minutes for the GitHub API limit', flush=True)
        time.sleep(wait)
    if r.returncode != 0:
        print(f'  skipped {repo}#{number}: {r.stderr.strip()[:160]}', flush=True)
        return None
    json.dump(json.loads(r.stdout), open(path, 'w', encoding='utf-8'))
    return json.loads(r.stdout)


def take_until(order, sizes, budget):
    """Files in this order while they fit in the budget, so a baseline never reads more than Looksie did."""
    got, used = set(), 0
    for f in order:
        if used + sizes[f] > budget:
            break
        got.add(f)
        used += sizes[f]
    return got


def found_after(order, sizes, fixed, share):
    """Fixed files met after reading this share of the changed lines, in this order."""
    total = sum(sizes.values()) or 1
    used, got = 0, set()
    for f in order:
        if got and used + sizes.get(f, 0) > share * total:
            break
        used += sizes.get(f, 0)
        got.add(f)
    return len(fixed & got)


def reading_order(rv, sizes):
    """The order Looksie shows files: decisions (focus, then context), over-limit steps, quick looks, then set aside."""
    rank = lambda st: 0 if st['tier'] == 'decide' else 1 if st.get('overLimit') else 2
    steps = sorted(rv['steps'], key=rank)
    decided = [st for st in steps if st['tier'] == 'decide']
    # Every decision's focus first; folded files only after all of them (folded means "not asked to read").
    out = [f['path'] for st in decided for f in st['files'] if not f.get('context')]
    out += [f['path'] for st in decided for f in st['files'] if f.get('context')]
    out += [f['path'] for st in steps if st['tier'] != 'decide' for f in st['files']]
    return out + [p for p in sizes if p not in out]


SHARES = [0.1, 0.2, 0.5]


def score(pr, rv):
    sizes = {f['path']: f['additions'] + f['deletions'] for f in rv['files']}
    for s in rv['skipped']:
        sizes.setdefault(s['path'], s['lines'])
    total = sum(sizes.values()) or 1
    tier = {}
    for s in rv['skipped']:
        tier[s['path']] = 'skipped'
    order = []  # decision files, in the order Looksie shows them
    first_hit = None
    fixed = {c['path'] for c in pr['comments'] if c['outdated']}
    decisions = [st for st in rv['steps'] if st['tier'] == 'decide']
    for i, st in enumerate(rv['steps']):
        t = 'decision' if st['tier'] == 'decide' else ('over-limit' if st.get('overLimit') else 'quick-look')
        for f in st['files']:
            # A file folded as lower-risk context inside a decision is not part of what the decision asks you to read.
            tier[f['path']] = 'context' if t == 'decision' and f.get('context') else t
            if tier[f['path']] == 'decision':
                order.append(f['path'])
    for i, st in enumerate(decisions):
        if first_hit is None and any(f['path'] in fixed and not f.get('context') for f in st['files']):
            first_hit = i
    in_pr = {p for p in fixed if p in sizes}
    decided = {p for p, t in tier.items() if t == 'decision'}
    effort_lines = sum(sizes[p] for p in decided)
    effort = effort_lines / total
    # Baselines get the same number of lines to read.
    biggest = take_until(sorted(sizes, key=lambda p: -sizes[p]), sizes, effort_lines)
    paths = list(sizes)
    rnd = 0.0
    rng = random.Random(f"{pr['repo']}#{pr['number']}")
    for _ in range(RANDOM_RUNS):
        rng.shuffle(paths)
        got = take_until(paths, sizes, effort_lines)
        rnd += len(in_pr & got)
    fixed_comments = [c for c in pr['comments'] if c['outdated'] and c['path'] in sizes]
    order = reading_order(rv, sizes)
    curve = [found_after(order, sizes, in_pr, sh) for sh in SHARES]
    rnd_curve = [0.0] * len(SHARES)
    for _ in range(100):
        rng.shuffle(paths)
        for i, sh in enumerate(SHARES):
            rnd_curve[i] += found_after(paths, sizes, in_pr, sh) / 100
    return {
        'repo': pr['repo'], 'number': pr['number'], 'language': pr['language'], 'kind': pr['kind'], 'lines': total,
        'files': len(sizes), 'decisions': len(decisions), 'effort': effort,
        'fixed_files': len(in_pr), 'fixed_files_gone': len(fixed - set(sizes)),
        'hit': len(in_pr & decided), 'hit_biggest': len(in_pr & biggest), 'hit_random': rnd / RANDOM_RUNS,
        'fixed_comments': len(fixed_comments), 'comment_hit': sum(1 for c in fixed_comments if tier.get(c['path']) == 'decision'),
        'where': {t: sum(1 for p in in_pr if tier.get(p, 'not-shown') == t) for t in ['decision', 'context', 'over-limit', 'quick-look', 'skipped', 'not-shown']},
        'first_hit': first_hit,
        'curve': curve, 'curve_random': rnd_curve,
    }


def summary(rows, label):
    n = len(rows)
    if not n:
        return None
    ff = sum(r['fixed_files'] for r in rows) or 1
    return {
        'label': label, 'prs': n, 'fixed_files': sum(r['fixed_files'] for r in rows),
        'recall': sum(r['hit'] for r in rows) / ff,
        'recall_biggest': sum(r['hit_biggest'] for r in rows) / ff,
        'recall_random': sum(r['hit_random'] for r in rows) / ff,
        'comment_recall': sum(r['comment_hit'] for r in rows) / (sum(r['fixed_comments'] for r in rows) or 1),
        'effort_median': sorted(r['effort'] for r in rows)[n // 2],
        'effort_mean': sum(r['effort'] for r in rows) / n,
        'lines_read_share': sum(r['effort'] * r['lines'] for r in rows) / (sum(r['lines'] for r in rows) or 1),
        'decisions_median': sorted(r['decisions'] for r in rows)[n // 2],
        'decisions_per_kloc': 1000 * sum(r['decisions'] for r in rows) / (sum(r['lines'] for r in rows) or 1),
        'prs_with_a_hit': sum(1 for r in rows if r['hit'] > 0) / n,
        'first_hit_is_first_decision': sum(1 for r in rows if r['first_hit'] == 0) / n,
        'where': {t: sum(r['where'].get(t, 0) for r in rows) for t in ['decision', 'context', 'over-limit', 'quick-look', 'skipped', 'not-shown']},
        'found_after': {f'{int(100 * sh)}%': sum(r['curve'][i] for r in rows) / ff for i, sh in enumerate(SHARES)},
        'found_after_random': {f'{int(100 * sh)}%': sum(r['curve_random'][i] for r in rows) / ff for i, sh in enumerate(SHARES)},
    }


def main():
    if not os.path.exists(EXE):
        sys.exit('Build the tool first: cargo build --release -p looksie-cli')
    os.makedirs(CACHE + TAG, exist_ok=True)
    # Every sample file (sample.json, sample_big.json, ...), each pull request once.
    sample, seen = [], set()
    for name in sorted(n for n in os.listdir(OUT) if n.startswith('sample') and n.endswith('.json')):
        for pr in json.load(open(os.path.join(OUT, name), encoding='utf-8')):
            if (pr['repo'], pr['number']) not in seen:
                seen.add((pr['repo'], pr['number']))
                sample.append(pr)
    # Held out: a fixed third of the repositories, chosen by name. Tune on the rest; check these only at the end.
    split = os.environ.get('BENCH_SPLIT', 'all')
    held = lambda repo: sum(repo.encode()) % 3 == 0
    if split == 'tune':
        sample = [p for p in sample if not held(p['repo'])]
    elif split == 'test':
        sample = [p for p in sample if held(p['repo'])]
    rows = []
    for pr in sample:
        if AT and not pr.get('at'):
            continue
        rv = review(pr['repo'], pr['number'], pr.get('at') if AT else None)
        if rv is None:
            continue
        r = score(pr, rv)
        if r['fixed_files'] == 0:
            continue
        rows.append(r)
        print(f"{pr['repo']}#{pr['number']:<6} {r['hit']}/{r['fixed_files']} fixed files in decisions, reading {r['effort']:.0%} of the lines "
              f"(biggest-first {r['hit_biggest']}, random {r['hit_random']:.1f})", flush=True)
    groups = [('All', rows)]
    for key in ['kind', 'language']:
        vals = defaultdict(list)
        for r in rows:
            vals[r[key]].append(r)
        groups += [(f'{key}: {v}', rs) for v, rs in sorted(vals.items())]
    size = defaultdict(list)
    for r in rows:
        size['under 1,000 lines' if r['lines'] < 1000 else '1,000 to 5,000 lines' if r['lines'] < 5000 else 'over 5,000 lines'].append(r)
    groups += [(f'size: {k}', v) for k, v in size.items()]
    groups.append(('reading under half the lines', [r for r in rows if r['effort'] < 0.5]))
    sums = [s for s in (summary(rs, label) for label, rs in groups) if s]
    json.dump({'rows': rows, 'summary': sums}, open(os.path.join(OUT, f'results{TAG}.json'), 'w', encoding='utf-8'), indent=1)
    for s in sums:
        print(f"{s['label']:<28} {s['prs']:>3} PRs  recall {s['recall']:.0%}  (biggest {s['recall_biggest']:.0%}, random {s['recall_random']:.0%})  "
              f"reading {s['lines_read_share']:.0%} of lines, {s['decisions_median']} decisions median | after 20% of lines: {s['found_after']['20%']:.0%} vs random {s['found_after_random']['20%']:.0%}")


if __name__ == '__main__':
    main()
