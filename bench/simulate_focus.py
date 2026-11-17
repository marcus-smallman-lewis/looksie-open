"""Applies a focus rule with given weights to reviews made without it (BENCH_SHARE=1.01, nothing folded, is what
crates/looksie-engine/src/focus.rs ships; a lower share also folds a decision's low-risk files, which was dropped),
and scores before and after with score.py's own scoring, so a weight change can be checked without rebuilding.

    BENCH_SPLIT=tune python bench/simulate_focus.py bench/out/weights_at.json
"""
import json, math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
os.environ.setdefault('BENCH_AT', '1')
import score as S
import learn as L

SHARE = float(os.environ.get("BENCH_SHARE", "0.75"))


def focus(rv, w):
    info = {(f['pr']['owner'], f['pr']['repo'], f['pr']['number'], f['path']): f for f in rv['files']}
    out = json.loads(json.dumps(rv))
    for st in out['steps']:
        files = st['files']
        risk = []
        for sf in files:
            f = info.get((sf['pr']['owner'], sf['pr']['repo'], sf['pr']['number'], sf['path']))
            n = sf['additions'] + sf['deletions']
            risk.append(L.prob(w, L.features(f, st, n)) if f else 0.0)
        ctx = [False] * len(files)
        is_dec = st['tier'] == 'decide' or st.get('overLimit')
        total = sum(risk)
        if is_dec and st['flag'] == 'none' and len(files) > 2 and total > 0:
            acc = 0.0
            for i in sorted(range(len(files)), key=lambda i: (-risk[i], i)):
                if acc >= SHARE * total and not files[i].get('focus'):
                    ctx[i] = True
                acc += risk[i]
        per = [risk[i] / (files[i]['additions'] + files[i]['deletions'] + 10) for i in range(len(files))]
        order = sorted(range(len(files)), key=lambda i: (ctx[i], -per[i], i))
        st['files'] = [dict(files[i], context=ctx[i]) for i in order]
    return out


def main():
    w = json.load(open(sys.argv[1]))
    w = [w[n] for n in L.NAMES]
    split = os.environ.get('BENCH_SPLIT', 'tune')
    held = lambda repo: sum(repo.encode()) % 3 == 0
    sample, seen = [], set()
    for name in sorted(n for n in os.listdir(S.OUT) if n.startswith('sample') and n.endswith('.json')):
        for pr in json.load(open(os.path.join(S.OUT, name), encoding='utf-8')):
            if (pr['repo'], pr['number']) not in seen and pr.get('at'):
                seen.add((pr['repo'], pr['number']))
                if (split == 'tune') != held(pr['repo']) or split == 'all':
                    sample.append(pr)
    before, after = [], []
    for pr in sample:
        path = os.path.join(S.OUT, 'reviews_at_before_all', f"{pr['repo'].replace('/', '_')}_{pr['number']}.json")
        if not os.path.exists(path):
            continue
        rv = json.load(open(path, encoding='utf-8'))
        b = S.score(pr, rv)
        if b['fixed_files'] == 0:
            continue
        before.append(b)
        after.append(S.score(pr, focus(rv, w)))
    for label, rows in [('before', before), ('after', after)]:
        for s in [S.summary(rows, 'All'), S.summary([r for r in rows if r['lines'] >= 5000], 'over 5,000 lines')]:
            if s:
                print(f"{split} {label:<6} {s['label']:<18} {s['prs']:>3} PRs  recall {s['recall']:.1%} vs random {s['recall_random']:.1%} (margin {100 * (s['recall'] - s['recall_random']):+.1f})  reading {s['lines_read_share']:.1%}  after 20%: {s['found_after']['20%']:.1%} vs {s['found_after_random']['20%']:.1%}")


if __name__ == '__main__':
    main()
