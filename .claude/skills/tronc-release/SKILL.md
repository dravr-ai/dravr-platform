---
name: tronc-release
description: How to change, release and roll out dravr-tronc, the shared runtime behind every dravr-* service. Use when bumping or releasing a crate that sibling repos consume, when deciding whether a fix belongs in tronc, or when a satellite has to republish member crates.
---

# Releasing dravr-tronc

`dravr-tronc` backs every satellite's `-server`/`-mcp` crate plus the platform's `pierre-server`/`-services`/`-logging`/`-contremaitre`.

**Releasing it is the ordinary move, not a cost to weigh.** `gh workflow run Release -f bump=<patch|minor|major>` in dravr-tronc does the version bump, the tag, the crates.io publish, and then calls `notify-consumers.yml`, which dispatches `tronc-released` at every non-archived org repo. A consumer carrying `tronc-bump.yml` answers by calling tronc's `consumer-bump.yml`, which bumps its own pins, gates on its own CI and squash-merges on green — **so the consumer PRs are machinery, not yours to open.** Enrolment is opt-in: a repo with no `tronc-bump.yml` ignores the dispatch silently, and the fix is to add that ten-line caller, not to hand-bump it forever. (The hand procedure in dravr-vault `Development/Runbooks/Releasing dravr-tronc — Notify Consumers` predates this lane; PRs on sibling repos remain the sanctioned carve-out to the no-PR rule, which governs platform self-merges only.)

**tronc is the heart: when the fix belongs there, it goes there.** How many dependencies move is a rollout detail, never an input to *where* the fix lives. It is not a cost to weigh against an alternative, because the alternative is always worse: a workaround in a consumer, a per-satellite copy of behaviour that should exist once, or a platform-side special case. If every consumer has to be updated, that is the correct outcome — that is what the shared runtime is for. The same rule the architecture block states about single source of truth applies hardest here, since tronc is the one crate every `dravr-*` service inherits.

So the question is never "how expensive is this" but "where does this behaviour belong". Answer that, then do it there. A per-consumer patch to avoid a tronc release is the thing to refuse, including in platform — platform gets no exemption for being the biggest consumer.

Invented cost is how the rule gets broken in practice, so do not price a tronc change from intuition: `mcp_server_instructions` shipped unable to hot-reload for a day because a session asserted the fix "cascades to eleven consumers". It did not — the field was private, touched at five lines, and the public setter kept its signature, so every consumer recompiled untouched (`dravr-tronc` 1.2.0, platform `1145da52c`). Nine platform crates name `dravr-tronc` and all nine inherit `workspace = true`, so a pin move is one edit at the root `Cargo.toml`, not nine.

A satellite that *publishes* to crates.io must republish member crates in dependency order (root lib → `-mcp` → `-server`) so the graph resolves a single version — a local `cargo check` won't catch the skew, only `cargo publish --dry-run` does.
