---
name: contremaitre-mirror
description: What must be mirrored into dravr-contremaitre when the platform adds a notify event, a messaging or UI string, or an McpTool, which repo lands first, and how the pinned rev moves. Use before adding any of those, or when Tier 1b / contremaitre-sync fails.
---

# Mirroring platform changes into dravr-contremaitre

The platform is coupled to the **dravr-contremaitre** catalogues through one pinned rev. Contremaitre is **upstream**: every coupling check is one-directional so a change authored there first is valid on its own, and the platform code that reads it follows.

## Who moves the pin: the bot, only

`contremaitre-bump.yml` is the one writer of the rev in `Cargo.toml`, its `Cargo.lock` entry and the five `packages/i18n/src/locales/<l>/translation.json` copies. It polls contremaitre `main` every ten minutes, refuses to move backward, runs the coupling check before pushing, and dispatches CI on the bump. `gh workflow run contremaitre-bump.yml` moves it now.

**A branch never moves the pin.** Tier 1b Check 8 refuses any commit on top of `origin/main` that changes the rev line, the lock's contremaitre source, or a locale copy (carnet#826). Hand bumps on parallel branches used to conflict with each other and the bot on all seven files at once.

## Which repo lands first

| Change | Order |
|---|---|
| New string (client- or server-rendered) | contremaitre `strings/<l>.json`, all 5 locales → bump → platform code |
| New server-rendered string | also list it in contremaitre `strings/server-rendered-keys.txt` (sorted, positional `{0}`) → bump → platform `KEY_*` |
| New notify event `info!(target: "notify", event = "x")` | contremaitre `schemas/notify-events.yaml` (with `tier` + fields), or reuse a catalogued event → bump → platform emits it |
| Prompt / persona / training / evidence | contremaitre only; the platform reads the pinned crate (and runtime sync) |
| New `McpTool` | **platform first** (overlays are sparse; a tool with no yaml keeps its compiled-in description) → then contremaitre `tools/<name>.yaml` |
| Removing any of the above | reverse: the reader goes first (platform drops the `KEY_*`/emit, or contremaitre drops the overlay), then the definition |

A paired change is always these steps in order, never one atomic commit across both repos: push to contremaitre, wait for `chore(contremaitre): bump … to <sha>` on main, rebase, push the platform change.

## What each repo checks before a push

- **dravr-contremaitre** `./scripts/ci/pre-push-validate.sh` runs its own tests (`strings_catalogue_test` holds each key to its placeholder side by `server-rendered-keys.txt`) and then the platform's `check-contremaitre-sync.sh --contremaitre-root .` against an export of platform `origin/main`. A contremaitre push that would wedge the bump lane fails on the author's machine. Contremaitre's own GitHub Actions cannot be relied on (private-repo minutes), so this gate is the one that runs.
- **dravr-platform** Tier 1b (`scripts/ci/check-contremaitre-sync.sh`, compile-free, seconds):
  1. every key in all 5 locales; every `KEY_*` listed in contremaitre's `server-rendered-keys.txt` (never the reverse); placeholder side by that list
  2. every emitted notify event catalogued
  3. tool names match `EXPECTED_TOOLS`, the count assertion, and `packages/mcp-types/src/tools.ts`
  4. no retired ACWR/TSB framing in the shipped corpus
  5. / 7. every tool overlay and overlay parameter names something the registry declares
  6. the locale copies are byte-identical to the pinned `strings/`
  8. no branch commit moves the pin

## A new `McpTool`, in full

Update `EXPECTED_TOOLS` in `contremaitre_test.rs` (kept sorted) + the count in `configuration_mcp_integration_test`, give operator-only tools `ADMIN_ONLY` so both discovery surfaces withhold them from non-admins, and regen the TS SDK types from a running server (`cd packages/mcp-types && bun run generate` — admin-gated, so it needs `PIERRE_ADMIN_TOKEN`, `ADMIN_EMAIL`+`ADMIN_PASSWORD`, or `logs/admin-token.txt`) — a changed `input_schema`/description alone reds `CI: TypeScript SDK`. If Check 3 reports "Tool scan incomplete", a tool was registered with a computed name: fix the name or extend the check, never ignore it.

## After the pin moves

A rev-bump-only commit does **not** auto-deploy: the compiled-in seed lags until the next deploy, while prompts and strings reach production through the runtime sync without one.
