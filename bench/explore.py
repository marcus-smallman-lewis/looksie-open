"""Tries ideas on the cached reviews without re-running the engine: which files inside a decision should be read first?

    python bench/explore.py            prints recall against reading for each idea, with random reading as the yardstick

An idea picks a set of "focus" files per pull request. Score: share of fixed files in focus, share of lines in focus,
and what random reading of the same number of lines finds. Only the tuning repositories are used (see score.py).
"""
import json, os, random, re
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, 'out')
held = lambda repo: sum(repo.encode()) % 3 == 0
SPLIT = os.environ.get('BENCH_SPLIT', 'tune')

TEST = re.compile(r'(^|/)(tests?|__tests__|spec|specs|testing)(/|$)|[._-](test|spec)s?\.[a-z]+$|_test\.go$|Tests?\.cs$|Test\.java$|(^|/)test_[^/]*\.py$', re.I)
DOCS = re.compile(r'\.(md|mdx|rst|txt|adoc)$', re.I)


def load():
    sample = {}
    for n in os.listdir(OUT):
        if n.startswith('sample') and n.endswith('.json'):
            for p in json.load(open(os.path.join(OUT, n), encoding='utf-8')):
                sample[(p['repo'], p['number'])] = p
    prs = []
    for (repo, num), p in sample.items():
        if (SPLIT == 'tune' and held(repo)) or (SPLIT == 'test' and not held(repo)):
            continue
        path = os.path.join(OUT, os.environ.get('BENCH_REVIEWS', 'reviews'), f"{repo.replace('/', '_')}_{num}.json")
        if not os.path.exists(path):
            continue
        rv = json.load(open(path, encoding='utf-8'))
        sizes = {f['path']: f['additions'] + f['deletions'] for f in rv['files']}
        for s in rv['skipped']:
            sizes.setdefault(s['path'], s['lines'])
        fixed = {c['path'] for c in p['comments'] if c['outdated'] and c['path'] in sizes}
        if fixed:
            prs.append((p, rv, sizes, fixed))
    return prs


def evaluate(name, pick, prs):
    hit = tot = rnd = 0.0
    read = lines = 0
    big_hit = big_tot = big_rnd = 0.0
    big_read = big_lines = 0
    for p, rv, sizes, fixed in prs:
        focus = pick(rv, sizes)
        e = sum(sizes.get(f, 0) for f in focus)
        t = sum(sizes.values()) or 1
        h = len(fixed & focus)
        paths = list(sizes)
        rng = random.Random(p['number'])
        acc = 0
        for _ in range(100):
            rng.shuffle(paths)
            got, used = set(), 0
            for f in paths:
                if used >= e:
                    break
                got.add(f)
                used += sizes[f]
            acc += len(fixed & got)
        hit += h; tot += len(fixed); rnd += acc / 100; read += e; lines += t
        if t >= 2000:
            big_hit += h; big_tot += len(fixed); big_rnd += acc / 100; big_read += e; big_lines += t
    out = f"{name:<34} all: {hit / tot:5.1%} vs random {rnd / tot:5.1%}, reading {read / lines:5.1%}"
    if big_tot:
        out += f" | 2k+ lines: {big_hit / big_tot:5.1%} vs random {big_rnd / big_tot:5.1%}, reading {big_read / big_lines:5.1%}"
    print(out)


def decision_files(rv):
    return [(s, [f['path'] for f in s['files']]) for s in rv['steps'] if s['tier'] == 'decide']


def current(rv, sizes):
    return {f for _, fs in decision_files(rv) for f in fs}


def kind_of(rv):
    return {f['path']: f for f in rv['files']}


def focus_by(rule, cap=None):
    def pick(rv, sizes):
        info = kind_of(rv)
        out = set()
        for s, fs in decision_files(rv):
            chosen = [f for f in fs if f in info and rule(info[f], s)]
            if not chosen:
                chosen = sorted(fs, key=lambda f: -sizes.get(f, 0))[:1]
            if cap:
                chosen = sorted(chosen, key=lambda f: -sizes.get(f, 0))[:cap]
            out |= set(chosen)
        return out
    return pick


def main():
    prs = load()
    print(f'{len(prs)} pull requests ({SPLIT})')
    evaluate('current decisions', current, prs)
    evaluate('focus: edited existing code', focus_by(lambda f, s: f['kind'] != 'added' and f['deletions'] > 0), prs)
    evaluate('focus: not tests or docs', focus_by(lambda f, s: not TEST.search(f['path']) and not DOCS.search(f['path'])), prs)
    evaluate('focus: biggest 3 per decision', focus_by(lambda f, s: True, cap=3), prs)
    evaluate('focus: biggest 1 per decision', focus_by(lambda f, s: True, cap=1), prs)
    evaluate('focus: edited, biggest 3', focus_by(lambda f, s: f['kind'] != 'added' and f['deletions'] > 0, cap=3), prs)
    code = lambda f: not TEST.search(f['path']) and not DOCS.search(f['path'])
    evaluate('focus: code, edited', focus_by(lambda f, s: code(f) and f['kind'] != 'added'), prs)
    evaluate('focus: code, edited or flagged', focus_by(lambda f, s: code(f) and (f['kind'] != 'added' or s['flag'] != 'none')), prs)
    # Which steps become decisions: the same number, chosen by score per line read.
    for name, key in [('rank: score / log2(lines)', lambda s, n: s['score'] / __import__('math').log2(n + 2)),
                      ('rank: score / sqrt(lines)', lambda s, n: s['score'] / (n + 1) ** 0.5)]:
        def pick(rv, sizes, key=key):
            cands = [s for s in rv['steps'] if s['tier'] == 'decide' or s.get('overLimit')]
            k = sum(1 for s in rv['steps'] if s['tier'] == 'decide')
            keep = [s for s in cands if s['flag'] != 'none']
            rest = sorted([s for s in cands if s['flag'] == 'none'], key=lambda s: -key(s, sum(sizes.get(f['path'], 0) for f in s['files'])))
            chosen = keep + rest[:max(0, k - len(keep))]
            return {f['path'] for s in chosen for f in s['files']}
        evaluate(name, pick, prs)
        def pick2(rv, sizes, key=key):
            info = kind_of(rv)
            fs = pick(rv, sizes)
            focus = {f for f in fs if f in info and code(info[f])}
            return focus or fs
        evaluate(name + ' + code focus', pick2, prs)
    # What kinds of files hold the fixes, per 1,000 lines? (higher = more worth reading)
    dens = defaultdict(lambda: [0, 0])
    for p, rv, sizes, fixed in prs:
        info = kind_of(rv)
        for f, n in sizes.items():
            k = 'test' if TEST.search(f) else 'docs' if DOCS.search(f) else (info[f]['kind'] if f in info else 'skipped')
            dens[k][0] += f in fixed
            dens[k][1] += n
    for k, (h, n) in sorted(dens.items()):
        print(f'  {k:<10} {h:>4} fixed files in {n:>9,} lines = {1000 * h / max(n, 1):.2f} per 1,000 lines')


if __name__ == '__main__':
    main()
