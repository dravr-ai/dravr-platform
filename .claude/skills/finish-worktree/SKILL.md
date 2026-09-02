---
name: finish-worktree
description: Completes feature branch work by rebasing, pushing, monitoring CI, and squash merging to main
user-invocable: true
---

# Finish Worktree Skill

**CLAUDE: When this skill is invoked with `/finish-worktree`, immediately run:**
```bash
./.claude/skills/finish-worktree/finish-worktree.sh
```
**Then watch CI, and once every lane is green, land the branch with `merge-and-cleanup.sh`.**

## Purpose
Lands a feature branch the way this repo lands everything: rebase onto `origin/main`, the
local pre-push gate, a push, CI green, then a squash merge that advances the one shared
`main` ref, followed by cleanup of the branch and its worktree. No pull request.

## Usage
```bash
/finish-worktree
```

## Workflow Steps

### Step 1: Rebase, gate, push (in the feature worktree)
```bash
./.claude/skills/finish-worktree/finish-worktree.sh
```

The script refuses a detached HEAD, `main`, and uncommitted changes; fetches and rebases
onto `origin/main` (stopping on a conflict for you to resolve); runs
`./scripts/ci/pre-push-validate.sh`, the only local gate, which writes the per-commit
marker the pre-push hook checks; pushes with `--force-with-lease`; and saves the branch
and worktree for Step 4.

### Step 2: Watch CI
Use the first available method, and **never ask for a GitHub token**:
1. `gh run list --branch <branch>` and `gh run view <id>` for one run; re-check on a
   schedule (`ScheduleWakeup`), never `gh run watch` and no loop under 60 seconds.
2. GitHub MCP tools (`mcp__github__*`) for anything that is not a listing.
3. WebFetch `https://github.com/dravr-ai/dravr-platform/actions?query=branch%3A<branch>`.

Wait until every lane is terminal and green. A `feature/*` ref runs the full PostgreSQL
suite; a `fix/*` ref runs only the 8-file smoke, so green there is not a full verdict.
A cancelled run is not a verdict either.

### Step 3: If CI fails
Fix the cause locally, validate the crate you touched, commit, and push again:
```bash
cargo clippy -p <crate> --all-targets --all-features -- -D warnings   # the crate you changed
cargo test --test <file> <name>                                        # the test that failed
git add <the files you changed>
git commit
./scripts/ci/pre-push-validate.sh && git push --force-with-lease origin <branch-name>
```
Do not run the full-workspace clippy locally as a gate; CI's `clippy` job does that on
every push. Repeat until every lane is green.

### Step 4: Squash merge and cleanup (from the main worktree, on `main`)
```bash
cd /path/to/main/worktree
./.claude/skills/finish-worktree/merge-and-cleanup.sh -m "<subject>

<body: what changed and why>"
```

No branch arguments needed: the script reads what Step 1 saved (`-F <file>` also works;
an interactive run with neither opens the editor prefilled with the branch's commit
subjects). It merges exactly the pushed `origin/<branch>`, the ref CI validated.

It refuses, with the reason printed:
- when it is not on `main` in the main worktree;
- when uncommitted changes in the main worktree overlap the branch's files (someone's
  work in progress would be swallowed; non-overlapping work is left alone, because only
  the staged paths are committed);
- when local `main` cannot fast-forward to `origin/main` (unpushed commits or a
  divergence need a person);
- when `origin/main` moved while the gate ran, or the push is rejected: the squash
  commit stays on local `main`, the recovery commands are printed, and **nothing is
  cleaned up**. Cleanup runs only after the push is confirmed, and the remote branch is
  deleted last.

The commit message is yours alone: no trailers, no attribution. The commit-msg hook
enforces a subject of at most 72 characters followed by a blank line.

Cleanup: `submodule deinit` then `git worktree remove --force` (a worktree from
`create-worktree` carries the `.build` submodule), `git branch -D` (a squash is not seen
as a merge), and `git push origin --delete` last.

### Step 5: Watch main
The push is the start of validation, not the end. Watch main's lanes for the landed
commit until they are terminal; a red one is yours to fix in the same session.

## Complete Example Session
```bash
# In the feature worktree
./.claude/skills/finish-worktree/finish-worktree.sh

# Watch CI (Step 2). If red, fix and push again (Step 3). Once green:
cd /path/to/main/worktree
./.claude/skills/finish-worktree/merge-and-cleanup.sh -m "feat(sdk): the bridge speaks MCP 2026-07-28 to Dravr

The Dravr leg moves onto a stateless client; the host side stays on the official SDK."

# Then watch main (Step 5).
```

## Related Skills
- `create-worktree` - Creates worktree with environment setup
