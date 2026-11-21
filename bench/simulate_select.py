"""Tries other ways of choosing which steps become decisions, on reviews already made, without rebuilding the engine.
Same number of decisions per pull request; what is never held back (rule findings, sensitive paths, data, build) stays.

    BENCH_SPLIT=tune python bench/simulate_select.py
"""
import json, math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
os.environ.setdefault('BENCH_AT', '1')
import score as S
import learn as L

REVIEWS = os.environ.get('BENCH_REVIEWS', 'reviews_at_v11_all')
W = json.load(open(os.path.join(S.OUT, 'weights_at.json')))
W = [W[n] for n in L.NAMES]
CAP_FROM = int(os.environ.get('BENCH_CAP_FROM', '0'))
PROTECT = ('Touches', 'Changes the database', 'Changes how it builds')


def protected(st):
    return st['flag'] != 'none' or any(r.startswith(PROTECT) for r in st['reasons'])


def reselect(rv, key, cap=None):
    """Keep the same number of decisions, chosen by `key` among steps that scored like decisions."""
    sizes = {f['path']: f['additions'] + f['deletions'] for f in rv['files']}
    total = sum(sizes.values()) or 1
    lines = lambda st: sum(sizes.get(f['path'], 0) for f in st['files'])
    cands = [st for st in rv['steps'] if st['tier'] == 'decide' or st.get('overLimit')]
    k = sum(1 for st in rv['steps'] if st['tier'] == 'decide')
    keep = [st for st in cands if protected(st)]
    rest = sorted([st for st in cands if not protected(st)], key=lambda st: -key(st, lines(st), rv))
    chosen = {st['id'] for st in keep}
    used = sum(lines(st) for st in keep)
    for st in rest:
        if len(chosen) >= max(k, len(keep)):
            break
        if cap and total >= CAP_FROM and used + lines(st) > cap * total and len(chosen) > 0:
            continue
        chosen.add(st['id'])
        used += lines(st)
    out = json.loads(json.dumps(rv))
    for st in out['steps']:
        if st['id'] in chosen:
            st['tier'] = 'decide'
        elif st['tier'] == 'decide' or st.get('overLimit'):
            st['tier'] = 'glance'
            st['overLimit'] = True
    # Decisions first, in their reading order; the engine's order is kept otherwise.
    out['steps'] = [st for st in out['steps'] if st['tier'] == 'decide'] + [st for st in out['steps'] if st['tier'] != 'decide']
    return out


def file_risk(f, n):
    """As crates/looksie-engine/src/focus.rs: the file's own traits only (no step-wide terms)."""
    x = L.features(f, None, n)
    return L.prob([w if name not in ('in decision', 'sensitive', 'step score/100') else 0.0 for w, name in zip(W, L.NAMES)], x)


def risk_density(st, n, rv):
    """As triage.rs chooses decisions: the step's summed file risk per line read (+50 so tiny steps do not win)."""
    info = {(f['pr']['repo'], f['path']): f for f in rv['files']}
    r = sum(file_risk(info[(sf['pr']['repo'], sf['path'])], sf['additions'] + sf['deletions']) for sf in st['files'] if (sf['pr']['repo'], sf['path']) in info)
    return r / (n + 50)


VARIANTS = {
    'today (score)': None,
    'file risk per line': risk_density,
}


def main():
    split = os.environ.get('BENCH_SPLIT', 'tune')
    held = lambda repo: sum(repo.encode()) % 3 == 0
    sample, seen = [], set()
    for name in sorted(n for n in os.listdir(S.OUT) if n.startswith('sample') and n.endswith('.json')):
        for pr in json.load(open(os.path.join(S.OUT, name), encoding='utf-8')):
            if (pr['repo'], pr['number']) in seen or not pr.get('at'):
                continue
            seen.add((pr['repo'], pr['number']))
            if split == 'all' or (split == 'tune') != held(pr['repo']):
                sample.append(pr)
    caps = [None] + [float(c) for c in os.environ.get('BENCH_CAPS', '').split(',') if c]
    for name, key in VARIANTS.items():
        for cap in (caps if key else [None]):
            rows = []
            for pr in sample:
                path = os.path.join(S.OUT, REVIEWS, f"{pr['repo'].replace('/', '_')}_{pr['number']}.json")
                if not os.path.exists(path):
                    continue
                rv = json.load(open(path, encoding='utf-8'))
                r = S.score(pr, reselect(rv, key, cap) if key else rv)
                if r['fixed_files']:
                    rows.append(r)
            for label, rs in [('all', rows), ('under 1,000', [r for r in rows if r['lines'] < 1000]), ('1,000-5,000', [r for r in rows if 1000 <= r['lines'] < 5000]), ('5,000+', [r for r in rows if r['lines'] >= 5000])]:
                s = S.summary(rs, label)
                print(f"{split} {name + (f' cap {cap:.0%}' if cap else ''):<32} {label:<13} {s['prs']:>3} PRs  in decisions {s['recall']:.1%} vs random {s['recall_random']:.1%} (margin {100 * (s['recall'] - s['recall_random']):+.1f})  reading {s['lines_read_share']:.1%}  first 20%: {s['found_after']['20%']:.1%}")


if __name__ == '__main__':
    main()
