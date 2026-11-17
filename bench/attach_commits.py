"""Adds to every sampled pull request the commit its reviewer first commented on, as `at`.

    python bench/attach_commits.py

score.py with BENCH_AT=1 then reviews each pull request as it was at that commit, so Looksie never sees the fixes the
reviewers asked for (the final version already has them, which would flatter any ranking that uses file size).
"""
import json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import collect as C

OUT = C.OUT


def main():
    for name in sorted(n for n in os.listdir(OUT) if n.startswith('sample') and n.endswith('.json')):
        path = os.path.join(OUT, name)
        sample = json.load(open(path, encoding='utf-8'))
        changed = 0
        for pr in sample:
            if pr.get('at'):
                continue
            items = C.api(f"repos/{pr['repo']}/pulls/{pr['number']}/comments?per_page=100") or []
            first = None
            for c in items:
                user = c.get('user') or {}
                if user.get('type') == 'Bot' or user.get('login', '').endswith('[bot]') or user.get('login') == pr['author']:
                    continue
                if c.get('in_reply_to_id') or c.get('subject_type') == 'file':
                    continue
                if not (c.get('line') is None and c.get('original_line') is not None):
                    continue
                if first is None or c['created_at'] < first['created_at']:
                    first = c
            if first:
                pr['at'] = first['original_commit_id']
                pr['at_time'] = first['created_at']
                changed += 1
            json.dump(sample, open(path, 'w', encoding='utf-8'), indent=1)
        print(f'{name}: {changed} pull requests given their review commit', flush=True)


if __name__ == '__main__':
    main()
