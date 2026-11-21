# Looksie's benchmark

How often do the files real reviewers asked to have fixed end up in Looksie's decisions? Results and method:
[looksie.dev/benchmarks.html](https://looksie.dev/benchmarks.html).

- `prs.csv`: every pull request scored: repository, number, language, whether an agent wrote it, its size, the commit
  it was reviewed at (the one its reviewer first commented on), tuning or held-out, how many files needed a fix, and
  how many of those were in a decision.
- `collect.py`: picks merged pull requests from public repositories with GitHub's CLI, and records the files where a
  person's inline review comment later went out of date (the code under it changed). `attach_commits.py` adds the
  commit each reviewer first commented on.
- `score.py`: runs Looksie on each pull request as it was at that commit and compares its decisions with reading the
  same number of lines in random order, or biggest files first. `learn.py` fits the weights that order files inside a
  step on the tuning repositories; `simulate_focus.py` tries such rules without rebuilding (it can also fold files,
  an idea that was dropped); `simulate_select.py` tries ways of choosing which steps become decisions, and `crossval.py` checks them with three-fold cross-validation by repository; `explore.py` holds shared loading code; `make_page.py` writes the benchmarks page.
  The weights were fitted on 21 of the 34 repositories and never on the other 13.

Scoring needs Looksie's command-line tool, which links the closed engine, so it is not in this repository; the list
of pull requests and `collect.py` can be reused to test any other review tool the same way.
