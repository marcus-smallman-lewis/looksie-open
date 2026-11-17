"""Builds the benchmark sample: merged pull requests from public repositories, with the review comments that were fixed.

    python bench/collect.py            writes bench/out/sample.json

The answer key for each pull request is the set of files where a human reviewer (not a bot, not the author) left an
inline comment that GitHub later marked outdated: the code under the comment changed after it was written. That is the
usual sign that a review comment led to a fix. Uses the GitHub CLI (`gh`) and its sign-in.
"""
import json, os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, 'out')

# Popular repositories across languages that review on GitHub. Agent-heavy ones are marked.
REPOS = [
    ('python', 'home-assistant/core'), ('python', 'pandas-dev/pandas'), ('python', 'astral-sh/uv'),
    ('typescript', 'microsoft/vscode'), ('typescript', 'vercel/next.js'), ('javascript', 'facebook/react'),
    ('go', 'grafana/grafana'), ('go', 'hashicorp/terraform'), ('go', 'kubernetes/kubernetes'),
    ('rust', 'astral-sh/ruff'), ('rust', 'tokio-rs/tokio'), ('rust', 'bevyengine/bevy'),
    ('java', 'apache/kafka'), ('java', 'spring-projects/spring-boot'),
    ('csharp', 'dotnet/aspnetcore'), ('csharp', 'dotnet/runtime'),
    ('ruby', 'rails/rails'), ('php', 'laravel/framework'), ('cpp', 'microsoft/terminal'),
]
# Popular repositories known for large pull requests, for the big-PR batch (BENCH_SET=big).
BIG_REPOS = [
    ('typescript', 'elastic/kibana'), ('typescript', 'n8n-io/n8n'), ('typescript', 'PostHog/posthog'), ('typescript', 'supabase/supabase'),
    ('typescript', 'calcom/cal.com'), ('typescript', 'microsoft/vscode'), ('typescript', 'vercel/next.js'), ('javascript', 'mastodon/mastodon'),
    ('python', 'getsentry/sentry'), ('python', 'apache/airflow'), ('python', 'langchain-ai/langchain'), ('python', 'home-assistant/core'),
    ('python', 'pytorch/pytorch'), ('go', 'grafana/grafana'), ('go', 'cockroachdb/cockroach'), ('go', 'prometheus/prometheus'),
    ('go', 'kubernetes/kubernetes'), ('rust', 'zed-industries/zed'), ('rust', 'denoland/deno'), ('rust', 'astral-sh/uv'),
    ('rust', 'bevyengine/bevy'), ('csharp', 'dotnet/maui'), ('csharp', 'dotnet/efcore'), ('csharp', 'microsoft/PowerToys'),
    ('csharp', 'dotnet/aspnetcore'), ('java', 'elastic/elasticsearch'), ('java', 'apache/kafka'), ('kotlin', 'JetBrains/kotlin'),
    ('cpp', 'microsoft/terminal'), ('ruby', 'rails/rails'), ('php', 'laravel/framework'), ('zig', 'oven-sh/bun'),
]
if os.environ.get('BENCH_SET') == 'big':
    REPOS = BIG_REPOS
AGENT_AUTHORS = ['app/copilot-swe-agent']
PER_REPO = int(os.environ.get('BENCH_PER_REPO', 6))
AGENT_PER_REPO = 3
MIN_LINES, MAX_LINES, MIN_FILES = int(os.environ.get('BENCH_MIN_LINES', 150)), 30000, 4


def wait_for_limit():
    """Sleep until GitHub's hourly limit resets (checking the limit does not count against it)."""
    out = subprocess.run(['gh', 'api', 'rate_limit', '-q', '[.resources.core.reset, .resources.search.reset] | max'], capture_output=True, text=True).stdout.strip()
    wait = max(60, int(out) - int(time.time()) + 20) if out.isdigit() else 600
    print(f'  waiting {wait // 60} minutes for the GitHub API limit', flush=True)
    time.sleep(wait)


def gh(*args):
    for attempt in range(6):
        r = subprocess.run(['gh', *args], capture_output=True, text=True, encoding='utf-8')
        if r.returncode == 0:
            return r.stdout
        if 'rate limit' in r.stderr.lower() or 'secondary rate' in r.stderr.lower():
            wait_for_limit()
            continue
        return None
    return None


def api(path):
    out = gh('api', '--paginate', '--slurp', path)
    if out is None:
        return None
    try:
        return [item for page in json.loads(out) for item in page]
    except Exception:
        return None


def fixed_comments(repo, number, author):
    items = api(f'/repos/{repo}/pulls/{number}/comments?per_page=100') or []
    keep = []
    for c in items:
        user = c.get('user') or {}
        if user.get('type') == 'Bot' or user.get('login', '').endswith('[bot]') or user.get('login') == author:
            continue
        if c.get('in_reply_to_id'):
            continue
        if c.get('subject_type') == 'file':
            continue
        # Outdated: the line it was written on no longer exists in the final diff (GitHub keeps original_line).
        outdated = c.get('line') is None and c.get('original_line') is not None
        keep.append({'path': c['path'], 'outdated': outdated, 'reviewer': user.get('login'), 'body': (c.get('body') or '')[:200]})
    return keep


def candidates(repo, author=None):
    args = ['search', 'prs', '--repo', repo, '--merged', '--limit', os.environ.get('BENCH_SEARCH', '60'), '--json', 'number', '--sort', 'updated']
    if author:
        args += ['--author', author]
    else:
        args += ['comments:>3']
    out = gh(*args)
    return [p['number'] for p in json.loads(out)] if out else []


def main():
    os.makedirs(OUT, exist_ok=True)
    name = os.environ.get('BENCH_OUT', 'sample.json')
    path = os.path.join(OUT, name)
    # Resume: keep what an earlier run found, and save after every pull request.
    sample = json.load(open(path, encoding='utf-8')) if os.path.exists(path) else []
    have = {(p['repo'], p['number']) for p in sample}
    for lang, repo in REPOS:
        for kind, author in [('human', None)] + [('agent', a) for a in AGENT_AUTHORS]:
            want = PER_REPO if kind == 'human' else AGENT_PER_REPO
            got = sum(1 for p in sample if p['repo'] == repo and p['kind'] == kind)
            if got >= want:
                continue
            for n in candidates(repo, author):
                if (repo, n) in have:
                    continue
                if got >= want:
                    break
                meta = gh('pr', 'view', str(n), '-R', repo, '--json', 'additions,deletions,changedFiles,author,title,createdAt')
                if not meta:
                    continue
                m = json.loads(meta)
                lines = m['additions'] + m['deletions']
                if not (MIN_LINES <= lines <= MAX_LINES and m['changedFiles'] >= MIN_FILES):
                    continue
                login = m['author']['login']
                if kind == 'human' and ('copilot' in login.lower() or login.endswith('[bot]')):
                    continue
                comments = fixed_comments(repo, n, login)
                fixed = [c for c in comments if c['outdated']]
                if not fixed:
                    continue
                sample.append({'repo': repo, 'number': n, 'language': lang, 'kind': kind, 'author': login, 'title': m['title'],
                               'lines': lines, 'files': m['changedFiles'], 'created': m['createdAt'], 'comments': comments})
                got += 1
                have.add((repo, n))
                json.dump(sample, open(path, 'w', encoding='utf-8'), indent=1)
                print(f'{repo}#{n} {kind} {lines} lines, {len(fixed)} fixed comments', flush=True)
    json.dump(sample, open(path, 'w', encoding='utf-8'), indent=1)
    print(f'{len(sample)} pull requests', flush=True)


if __name__ == '__main__':
    main()
