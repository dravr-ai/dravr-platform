---
name: review-standards
description: Review a finished diff against docs/coding-standards.md and the carnet issue it implements, in fix mode, in a fresh subagent context. Run before every squash to main, and on a direct bug-fix push whenever it can wait. Prints the Reviewed-Standards trailer the landing commit carries.
argument-hint: "[<base>...<head>]  (default origin/main...HEAD)"
user-invocable: true
---

# Review against the coding standards

The implementing session makes the code work; this pass makes it good. The standards live in
`docs/coding-standards.md` and nowhere in the implementer's context (carnet#660), so this pass is
the only place most of them are read. JF's call (2026-09-30): it runs before **every squash** to
main.

## 1. Collect the inputs (main session)

The range is the skill's argument, or `origin/main...HEAD` when none is given.

```bash
RANGE=origin/main...HEAD    # or the argument
git fetch -q origin
git diff --stat "$RANGE"
git log --format='%h %s%n%b' "$RANGE" | grep -o 'carnet#[0-9]\+' | sort -u    # the issues it implements
```

For each carnet issue, read its body and every comment. "Directive from JF" comments are
requirements. Use REST, never `gh issue` (GraphQL is blocked in cloud sessions):

```bash
gh api repos/dravr-ai/dravr-carnet/issues/<n> --jq .body
gh api repos/dravr-ai/dravr-carnet/issues/<n>/comments --jq '.[].body'
```

## 2. Run the review in a fresh context

Spawn one `general-purpose` subagent. Its context holds only what the review needs, which is why
it can apply every standard reliably. Give it:

- the range (`git diff <range>` is its input; it reads surrounding code only for context),
- the path `docs/coding-standards.md`, to be read in full before judging anything,
- the issue bodies and directives from step 1 (the spec axis: does the diff do what was asked,
  all of it, and nothing else?),
- these instructions:
  - **Fix, don't comment.** Edit the working tree for every violation you are confident about.
    A finding becomes a question only when fixing it needs a decision (a product choice, a
    scope call, a trade-off the standards don't settle).
  - Touch only code the diff added or changed, plus what a fix strictly requires.
  - Never weaken a test or an assertion to make something pass.
  - Report: each fix (file:line, rule it enforces), each open question, and the rules checked
    that found nothing.

## 3. Validate and commit the fixes (main session)

Read the subagent's diff; revert anything outside the rules above. Validate what it touched
(per-crate clippy, the targeted tests, the frontend/mobile tiers), then commit on the branch:

```bash
git commit -m "review: apply coding standards to <slice>"
```

Put each open question in front of ChefFamille; do not squash over an unanswered one.

## 4. The trailer

The landing commit ends with the trailer below: the squash message on the feature path, the
commit itself on a direct fix. The value pins which version of the standards the diff was
judged against.

```bash
printf 'Reviewed-Standards: %s\n' "$(git rev-parse --short=12 HEAD:docs/coding-standards.md)"
```

A commit already pushed without one caps `bilan` at 8. Review it (`/review-standards
<sha>~1...<sha>`), then land the follow-up with the trailer extended by the commits it covers:
`Reviewed-Standards: <standards sha> covers <short sha> <short sha>`. That clears the cap for
the named commits.

## Pilot

This runs as a two-week pilot (`feature-phases.yaml`, `agents-md-standards-review`). Record
each run's findings by standards section in the carnet#660 thread so the exit criteria can be
read off it: findings per slice, standards-related CI reds, post-squash rework.
