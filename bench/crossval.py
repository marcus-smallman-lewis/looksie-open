"""Three-fold cross-validation by repository: every pull request is scored by a rule whose weights never saw its
repository. More reliable than one held-out split when the folds are small.

    python bench/crossval.py
"""
import json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
os.environ.setdefault('BENCH_AT', '1')
os.environ.setdefault('BENCH_CAP_FROM', '1000')
import score as S
import learn as L
import simulate_select as SEL

REVIEWS = os.environ.get('BENCH_REVIEWS', 'reviews_at_v11_all')
fold = lambda repo: sum(repo.encode()) % 3


def load():
    sample, seen = [], set()
    for name in sorted(n for n in os.listdir(S.OUT) if n.startswith('sample') and n.endswith('.json')):
        for pr in json.load(open(os.path.join(S.OUT, name), encoding='utf-8')):
            if (pr['repo'], pr['number']) in seen or not pr.get('at'):
                continue
            seen.add((pr['repo'], pr['number']))
            path = os.path.join(S.OUT, REVIEWS, f"{pr['repo'].replace('/', '_')}_{pr['number']}.json")
            if os.path.exists(path):
                sample.append((pr, json.load(open(path, encoding='utf-8'))))
    return sample


def training_rows(items):
    """The same features and labels as learn.py, for these pull requests."""
    data = []
    for pr, rv in items:
        sizes = {f['path']: f['additions'] + f['deletions'] for f in rv['files']}
        fixed = {c['path'] for c in pr['comments'] if c['outdated'] and c['path'] in sizes}
        if not fixed:
            continue
        step_of = {f['path']: s for s in rv['steps'] for f in s['files']}
        for f in rv['files']:
            data.append((L.features(f, step_of.get(f['path']), sizes.get(f['path'], 0)), 1.0 if f['path'] in fixed else 0.0))
    return data


VARIANTS = [('today (score)', None, None), ('file risk per line', 'risk', None), ('file risk per line, cap 60% from 1,000 lines', 'risk', 0.6), ('file risk per line, cap 50% from 1,000 lines', 'risk', 0.5)]


def main():
    items = load()
    rows = {name: [] for name, _, _ in VARIANTS}
    for k in range(3):
        train = [x for x in items if fold(x[0]['repo']) != k]
        test = [x for x in items if fold(x[0]['repo']) == k]
        w = L.fit(training_rows(train))
        SEL.W = w
        for name, key, cap in VARIANTS:
            for pr, rv in test:
                r = S.score(pr, SEL.reselect(rv, SEL.risk_density, cap) if key else rv)
                if r['fixed_files']:
                    rows[name].append(r)
        print(f'fold {k}: trained on {len(train)}, tested on {len(test)} pull requests', flush=True)
    out = {}
    for name, _, _ in VARIANTS:
        rs = rows[name]
        out[name] = {label: S.summary(part, label) for label, part in [('all', rs), ('under 1,000', [r for r in rs if r['lines'] < 1000]), ('1,000-5,000', [r for r in rs if 1000 <= r['lines'] < 5000]), ('5,000+', [r for r in rs if r['lines'] >= 5000])]}
        for label, part in [('all', rs), ('under 1,000', [r for r in rs if r['lines'] < 1000]), ('1,000-5,000', [r for r in rs if 1000 <= r['lines'] < 5000]), ('5,000+', [r for r in rs if r['lines'] >= 5000])]:
            s = S.summary(part, label)
            print(f"{name:<46} {label:<12} {s['prs']:>3} PRs  in decisions {s['recall']:.1%} vs random {s['recall_random']:.1%} (margin {100 * (s['recall'] - s['recall_random']):+.1f})  reading {s['lines_read_share']:.1%}  first 20%: {s['found_after']['20%']:.1%}")
    json.dump(out, open(os.path.join(S.OUT, 'crossval.json'), 'w'), indent=1)


if __name__ == '__main__':
    main()
