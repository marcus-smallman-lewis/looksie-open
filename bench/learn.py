"""Which file traits predict a reviewer's fix? Fits a small logistic regression on the tuning repositories (plain Python),
prints the weights, then checks an effort-aware focus on the held-out repositories.

    python bench/learn.py
"""
import json, math, os, random, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import explore as E

NAMES = ['bias', 'log lines', 'hunks 2-3', 'hunks 4+', 'new file', 'deletes most', 'mixed edit', 'test', 'docs',
         'config/data', 'sensitive', 'in decision', 'step score/100', 'small (<20)']
CONFIG = ('.json', '.yml', '.yaml', '.toml', '.xml', '.lock', '.ini', '.cfg', '.props', '.csproj', '.sln')


def features(f, step, n):
    h = len(f['hunks'])
    reasons = step['reasons'] if step else []
    return [1.0, math.log1p(n), 1.0 if 2 <= h <= 3 else 0.0, 1.0 if h >= 4 else 0.0, 1.0 if f['kind'] == 'added' else 0.0,
            1.0 if f['deletions'] > f['additions'] else 0.0, 1.0 if 0 < f['deletions'] <= f['additions'] else 0.0,
            1.0 if E.TEST.search(f['path']) else 0.0, 1.0 if E.DOCS.search(f['path']) else 0.0,
            1.0 if f['path'].lower().endswith(CONFIG) else 0.0,
            1.0 if any(r.startswith(('Touches', 'The new code mentions')) for r in reasons) else 0.0,
            1.0 if step and step['tier'] == 'decide' else 0.0, (step['score'] / 100.0) if step else 0.0, 1.0 if n < 20 else 0.0]


def rows(prs):
    out = []
    for p, rv, sizes, fixed in prs:
        step_of = {f['path']: s for s in rv['steps'] for f in s['files']}
        for f in rv['files']:
            out.append((features(f, step_of.get(f['path']), sizes.get(f['path'], 0)), 1.0 if f['path'] in fixed else 0.0))
    return out


def fit(data, l2=0.01, steps=3000, lr=0.3):
    w = [0.0] * len(NAMES)
    for _ in range(steps):
        g = [0.0] * len(w)
        for x, y in data:
            z = sum(a * b for a, b in zip(w, x))
            p = 1 / (1 + math.exp(-max(-30, min(30, z))))
            for i, xi in enumerate(x):
                g[i] += (p - y) * xi
        w = [wi - lr * (gi / len(data) + l2 * wi * (i > 0)) for i, (wi, gi) in enumerate(zip(w, g))]
    return w


def prob(w, x):
    return 1 / (1 + math.exp(-sum(a * b for a, b in zip(w, x))))


def main():
    E.SPLIT = 'tune'
    tune = E.load()
    w = fit(rows(tune))
    for n, v in sorted(zip(NAMES, w), key=lambda t: -abs(t[1])):
        print(f'  {n:<16} {v:+.2f}')
    json.dump(dict(zip(NAMES, w)), open(os.path.join(E.OUT, 'weights.json'), 'w'), indent=1)
    for split in ['tune', 'test']:
        E.SPLIT = split
        prs = E.load()
        print(f'--- {split}: {len(prs)} pull requests')
        E.evaluate('current decisions', E.current, prs)

        def focus(rv, sizes, t):
            step_of = {f['path']: s for s in rv['steps'] for f in s['files']}
            out = set()
            for s, fs in E.decision_files(rv):
                info = {f['path']: f for f in rv['files']}
                scored = [(prob(w, features(info[f], s, sizes.get(f, 0))), f) for f in fs if f in info]
                keep = {f for p, f in scored if p >= t}
                out |= keep or {max(scored)[1]} if scored else set()
            return out
        for t in [0.1, 0.15, 0.2, 0.25]:
            E.evaluate(f'decisions, focus p>={t}', lambda rv, sizes, t=t: focus(rv, sizes, t), prs)


if __name__ == '__main__':
    main()


def recall_at(order, sizes, fixed, share):
    total = sum(sizes.values()) or 1
    used, got = 0, set()
    for f in order:
        if used + sizes.get(f, 0) > share * total and got:
            break
        used += sizes.get(f, 0)
        got.add(f)
    return len(fixed & got)


def curves(w):
    import random as R
    for split in ['tune', 'test']:
        E.SPLIT = split
        prs = E.load()
        def tier_order(rv, sizes, key=None):
            seq = []
            for s in rv['steps']:
                fs = [f['path'] for f in s['files']]
                if key:
                    fs.sort(key=lambda f: key(rv, s, f, sizes))
                seq.append((0 if s['tier'] == 'decide' else 1 if s.get('overLimit') else 2, fs))
            out = [f for t, fs in sorted(seq, key=lambda x: x[0]) for f in fs]
            return out + [f for f in sizes if f not in out]
        info_cache = {}
        def p_of(rv, s, f, sizes):
            info = {x['path']: x for x in rv['files']}
            return prob(w, features(info[f], s, sizes.get(f, 0))) if f in info else 0.0
        strategies = {
            'random order': None,
            'Looksie today': lambda rv, sizes: tier_order(rv, sizes),
            'Looksie, risk per line inside steps': lambda rv, sizes: tier_order(rv, sizes, key=lambda rv, s, f, sz: -p_of(rv, s, f, sz) / (sz.get(f, 0) + 10)),
            'all files by risk per line': lambda rv, sizes: sorted(sizes, key=lambda f: -max([p_of(rv, s, f, sizes) for s in rv['steps'] if any(x['path'] == f for x in s['files'])] or [0]) / (sizes[f] + 10)),
            'decisions first, each by risk per line, code before tests': lambda rv, sizes: tier_order(rv, sizes, key=lambda rv, s, f, sz: ((1 if (E.TEST.search(f) or E.DOCS.search(f)) else 0), -p_of(rv, s, f, sz) / (sz.get(f, 0) + 10))),
        }
        print(f'--- {split}: {len(prs)} pull requests; share of fixed files found after reading 10% / 20% / 50% of the lines (2k+ line PRs in brackets)')
        for name, strat in strategies.items():
            res = {}
            for share in [0.1, 0.2, 0.5]:
                h = t = bh = bt = 0.0
                for p, rv, sizes, fixed in prs:
                    if strat is None:
                        rng = R.Random(p['number']); acc = 0
                        for _ in range(50):
                            o = list(sizes); rng.shuffle(o); acc += recall_at(o, sizes, fixed, share)
                        hit = acc / 50
                    else:
                        hit = recall_at(strat(rv, sizes), sizes, fixed, share)
                    h += hit; t += len(fixed)
                    if sum(sizes.values()) >= 2000:
                        bh += hit; bt += len(fixed)
                res[share] = (h / t, bh / max(bt, 1))
            print(f"  {name:<58} " + '  '.join(f"{a:5.1%} ({b:5.1%})" for a, b in res.values()))


if __name__ == '__main__' and os.environ.get('CURVES'):
    curves(json.load(open(os.path.join(E.OUT, 'weights.json'))) and [json.load(open(os.path.join(E.OUT, 'weights.json')))[n] for n in NAMES])
