# GitWell

[![crates.io](https://img.shields.io/crates/v/gitwell.svg)](https://crates.io/crates/gitwell)
[![docs.rs](https://img.shields.io/docsrs/gitwell)](https://docs.rs/gitwell)
[![license](https://img.shields.io/crates/l/gitwell.svg)](./LICENSE)

> Your git history is a journal you didn't know you were writing.

GitWell scans local git repositories and surfaces **abandoned work** — the
stale branches, forgotten stashes, orphaned commits, and unfinished intentions
that accumulate over time and quietly disappear from the mental model of
whoever last touched the repo.

It's not a linter. It's an archaeologist.

## Install

```sh
cargo install gitwell
```

Or build from source:

```sh
git clone https://github.com/davidcanhelp/GitWell
cd GitWell
cargo install --path .
```

**Requirements:** A working `git` binary on your `$PATH`. That's it — GitWell
has **zero crates.io dependencies** and uses only the Rust standard library.

## What it finds

| Scanner          | What it surfaces                                                        |
| ---------------- | ----------------------------------------------------------------------- |
| **Dormant Repos**  | Repositories with no commits on any branch in 6+ months.              |
| **Stale Branches** | Unmerged local branches untouched for 30+ days, with ahead/behind counts vs. the default branch. |
| **Stashes**        | Every stash, with age, message, and `(files, +ins/-dels)`.            |
| **Orphan Commits** | Commits in the reflog no longer reachable from any branch or tag.     |
| **WIP Markers**    | Commits whose subjects contain `WIP`, `TODO`, `FIXME`, `temp`, `hack`, `experiment`, `trying`, or `broken`. |

Findings are sorted oldest-first and color-coded by age:

- **red** — 1 year or older
- **yellow** — 6 months or older
- **green** — 30 days or older

## Sessions of abandoned work

Individual findings are often pieces of the same larger effort: a stale branch,
a stash from that same weekend, and three WIP commits all belong to "that auth
refactor I started in March." GitWell groups related findings into **sessions**
and prints a one-line narrative for each:

```
Sessions of Abandoned Work

  [9] You have 9 WIP commits in io from a single weekend in June 2021.
      Looks like a sprint that stalled.

  [6] In December 2017 – August 2025, you started a stellar effort across
      2 repos. 5 WIP commits and 1 dormant repo.
```

Sessions come from two stages of clustering:

1. **Per-repo time-gap sessions** — within each repo, findings are grouped
   into bursts; a new burst starts whenever the gap between consecutive
   findings exceeds `session_window_hours`.
2. **Cross-repo theme merge** — bursts from different repos merge when their
   keyword sets share at least two non-stopword tokens. Requiring two
   shared tokens keeps noise words like "broken" or "cleanup" from
   transitively linking thousands of unrelated commits.

Narratives are produced by `src/narrative.rs` from pure heuristics — no AI,
no ML — picking one of a handful of templates based on cluster shape (WIP
sprint, cross-repo effort, big stash, orphan graveyard, dormant sweep,
generic effort).

## Reports and trends

`gitwell report [PATH]` writes a standalone markdown file at
`<PATH>/.gitwell/report-YYYY-MM-DD.md`. It has an executive summary, the
sessions of abandoned work with their narratives, per-scanner findings, and
any triage history. It's the thing you'd share with a collaborator or read
once a month.

Every report run also appends a row to `<PATH>/.gitwell/history.json`
(`[{date, timestamp, repos, findings, sessions}]`). The next report uses
the most recent previous row to compute a delta and print a "Since last
scan" block at the top:

```markdown
## Since last scan

_2 days ago: 227 → 213 findings (-14), 32 → 29 sessions (-3)_

- Findings: **213** (net -14)
- Sessions: **29** (net -3)
- Repos: **28** (net +0)
```

## Git hook

Install a post-commit hook that nudges you when abandoned work piles up:

```sh
gitwell hook install
```

This writes (or appends to) `.git/hooks/post-commit` inside a clearly
delimited managed block — your existing hook content, if any, is
preserved. The hook embeds the absolute path to the `gitwell` binary you
ran `install` from, so it keeps working across shell sessions.

After each commit you'll see something like:

```
GitWell: 3 sessions, 11 findings across 1 repo
```

and silence the rest of the time. Uninstall with:

```sh
gitwell hook remove
```

which strips only the managed block, leaving any user-authored hook
content intact.

## Configuration

GitWell looks for an optional config file at, in order:

1. `.gitwell.toml` in the scanned directory
2. `~/.config/gitwell/config.toml`

The format is a tiny subset of TOML — flat key-value pairs and simple
string arrays, parsed by hand so GitWell stays dependency-free:

```toml
stale_days = 30              # threshold for stale branches
dormant_months = 6           # threshold for dormant repos
session_window_hours = 48    # clustering time window
ignore_repos = ["node_modules", ".cache"]
ignore_branches = ["dependabot/*", "renovate/*"]
```

Unknown keys are silently ignored, so older configs keep working as new
fields are added.

## Usage

```sh
# Scan a single repo or a directory of repos.
gitwell                         # current directory
gitwell ~/code/some-project
gitwell ~/code                  # all repos directly under ~/code

# One-line summary only (for shell scripts and git hooks).
gitwell --quiet ~/code

# Machine-readable output.
gitwell ~/code --json

# Walk through sessions interactively and decide what to do.
gitwell triage ~/code
gitwell triage ~/code --reset   # clear the decision log

# Dry-run queued actions (shows what would happen)…
gitwell execute ~/code
# …then actually apply them.
gitwell execute ~/code --confirm

# Write a markdown report to .gitwell/report-YYYY-MM-DD.md
# and append a row to .gitwell/history.json for trend tracking.
gitwell report ~/code

# Install a post-commit hook that nudges you after commits.
gitwell hook install
gitwell hook remove
```

### Sample output

```
GitWell — scanned 28 repos · 213 findings · 29 sessions

Sessions of Abandoned Work

  [9] You have 9 WIP commits in io from a single weekend in June 2021.
      Looks like a sprint that stalled.
      repos: io · label: sigpipe
      io  [4y] 6b0f0ee4 [trying] Trying bash on Windows
      io  [4y] 060ca740 [trying] Trying to satisfy sigpipe test in CI
      ...

  [6] In December 2017 – August 2025, you started a stellar effort across
      2 repos. 5 WIP commits and 1 dormant repo.
      repos: stellar-core, stellar-go · label: stellar
      stellar-go  [8y] 0cdf4695 [temp] temp
      stellar-go  [8y] 5106ba0d [hack] HACK: improve core elder query detection
      stellar-core [8mo] dormant
      ...

alpha /path/to/alpha
  Stale Branches (2)
    [7mo] feature/auth-v2 — WIP: auth refactor experiment (+1/-0)
    [7mo] feature/old-migration — temp migration hack (+1/-0)
  Stashes (1)
    [0h] stash@{0} — On main: WIP auth draft work (1 file, +1/-0)
```

The `--quiet` one-liner looks like:

```
GitWell: 5 sessions, 29 findings across 3 repos
```

and prints nothing at all when the repo is clean — ideal for a post-commit
hook that only speaks up when there's something to say.

## Triage and execute

`gitwell triage` walks through the sessions one at a time. For each, it shows
the narrative and finding list and prompts for a single keypress:

```
  [r]esume [a]rchive [d]elete [s]kip [q]uit:
```

- **resume** — flag for follow-up. For stashes, `git stash apply` at execute time.
- **archive** — for branches: `git tag archive/<name>` then delete. For stashes: save `git stash show -p` to `.gitwell/archives/<date>-<repo>-<summary>.patch` then drop.
- **delete** — `git branch -D` / `git stash drop`. Gated by a second `y/n` confirm.
- **skip** — record that the session was seen but take no action.
- **quit** — save progress and exit; re-running `gitwell triage` resumes where you left off.

Decisions are written incrementally to `<scan_path>/.gitwell/triage.json` so
a killed process doesn't lose earlier answers. Sessions already in the log
are skipped on re-run.

`gitwell execute` reads that same file and runs the queued actions. It's a
**dry run by default** — you have to pass `--confirm` to actually touch
anything. Destructive actions that find their target already gone (branch
deleted by hand, stash index shifted away) are warned about and the rest
of the queue keeps going. Stashes are located by commit SHA, not by
`stash@{N}`, so dropping stash@{2} doesn't break a later drop of what was
stash@{0}.

The interactive loop puts the terminal into `cbreak -echo` mode via `stty`
(no `libc` crate dependency). An RAII guard restores the original
terminal state on exit — press `q` to quit cleanly. If you Ctrl+C mid-session
and your terminal looks weird, `stty sane` restores it.

## Design

GitWell has **zero crates.io dependencies**. It uses only `std`.

Every git interaction goes through a thin wrapper over `std::process::Command`
in `src/git.rs` — GitWell never parses `.git` internals directly. If git can
tell you the answer, GitWell will ask git.

```
src/
├── main.rs              CLI entry point, subcommand dispatch, repo discovery
├── config.rs            Hand-written TOML config loader
├── git.rs               Thin wrapper around the `git` binary
├── util.rs              Shared helpers (time, globs, civil dates, ISO 8601)
├── json.rs              Minimal JSON parser + pretty printer
├── cluster.rs           Two-stage session clustering
├── narrative.rs         Template-based one-line summaries
├── report.rs            Terminal + JSON formatting
├── report_md.rs         Markdown report generator
├── trends.rs            .gitwell/history.json + delta computation
├── triage.rs            Interactive triage loop (stty cbreak/-echo)
├── triage_state.rs      .gitwell/triage.json load/save
├── execute.rs           Action runner (dry-run + --confirm)
├── hook.rs              post-commit hook install/remove
└── scanner/
    ├── mod.rs           Scanner trait, Finding enum, registry
    ├── branches.rs      Stale branches
    ├── stashes.rs       Stashes
    ├── orphans.rs       Reflog-only commits
    ├── wip.rs           WIP-marker commit subjects
    └── dormant.rs       Repos with no recent activity
```

Adding a new scanner means writing a module that implements the `Scanner`
trait, then adding it to `scanner::registry()`. That's the whole extension
point.

## License

MIT
