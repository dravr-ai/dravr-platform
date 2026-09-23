---
name: private-ci
description: Run CI or a release on a private dravr repo while the org's private-repo GitHub Actions minutes are exhausted — scan the history, make the repo public, run, make it private again; a red CI leaves it private until a new push. Use whenever a private repo's run sits queued or fails with "The job was not started because recent account payments have failed or your spending limit needs to be increased", before assuming a private repo's CI is broken, and before releasing any private satellite.
argument-hint: "check | scan <repo> <checkout> | run <repo> <checkout> [--release patch|minor|major]"
user-invocable: true
---

# Private-repo CI while the Actions minutes are out

**ChefFamille's procedure (carnet#534): public → CI → private.** The org is on
GitHub's free plan. When the private-repo Actions minutes run out, private repos
cannot run a single job — every run either sits queued or concludes `failure` with
no step executed. Public repos still run free. So a private repo's CI or release
runs by making the repo public for exactly that run.

**If CI fails, the repo goes back to private and stays private until a new push.**
You do not leave it public to debug. Fix the code, push, and run this again.

## Is the limit reached?

```bash
.agents/skills/private-ci/private-ci.sh check
```

Exit **3** and `⛔` when GitHub refused a private-repo job in the last 24 hours —
the refusal is the only direct evidence (GitHub publishes no remaining-minutes
figure). `✅` means no refusal was *seen*, which is not proof the limit is clear.

Signs you are hitting it without running the check:
- a private repo's run stays `queued` for tens of minutes while public repos run;
- a job shows **"The job was not started because recent account payments have
  failed or your spending limit needs to be increased"**, `failure`, zero steps.
  That is a bill, not a broken build — do not go looking for a code break.

## Running CI or a release

```bash
# CI on main
.agents/skills/private-ci/private-ci.sh run dravr-<x> ~/workspace/dravr-<x>
# CI, then a release if green
.agents/skills/private-ci/private-ci.sh run dravr-<x> ~/workspace/dravr-<x> --release minor
# see the plan without changing anything
.agents/skills/private-ci/private-ci.sh run dravr-<x> ~/workspace/dravr-<x> --dry-run
```

What `run` does, in order:

1. **Refuses dravr-carnet and dravr-vault** outright. Carnet issue bodies name
   security residuals; the vault is the team's notes. Neither is ever public.
2. **Scans the whole git history** (every commit, every branch) for secret-shaped
   content — private keys, cloud and GitHub tokens, Stripe keys, credential files,
   quoted password/token literals. Going public exposes all of it, and public repos
   are scraped within minutes: making the repo private again does **not** undo
   that. Any hit stops the run. Read every line; test fixtures and doc placeholders
   are fine (`--scan-reviewed` then), a real credential means **stop and tell
   ChefFamille** — rotating it comes before any flip.
3. Makes the repo public, and installs a trap that makes it **private again on any
   exit** — success, red CI, a failed step, Ctrl-C.
4. Cancels runs that were queued *before* the flip: GitHub decides billing at queue
   time, so they can never start. Then dispatches the workflow on `main`.
5. Waits for it. **Red → private, exit 1: stays private until a new push.**
6. With `--release`, dispatches the `Release` workflow and waits for it.
7. Waits until nothing it set off in the repo is still queued or running (an
   announce to the platform, a bump lane), then the trap makes it private.

Do everything that needs Actions in one run: a second flip is a second exposure,
not a smaller one.

## After it runs

- A release announced to dravr-platform is picked up by the platform's own lane
  (`bump-<name>.yml`, public, runs normally). Do not hand-bump that pin.
- The repo's visibility is the last line `run` prints. Confirm it reads `private`.
