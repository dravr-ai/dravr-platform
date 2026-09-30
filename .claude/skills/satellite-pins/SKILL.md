---
name: satellite-pins
description: How dravr-* satellite pins are declared and moved (satellites.toml, satellite-bump.yml, bump-<name>.yml, satellite-pin.sh). Use when adding, removing or changing a satellite pin, touching a bump lane, or diagnosing satellite drift.
---

# Satellite pins and bump lanes

Every satellite pin is declared once in **`satellites.toml`** at the repo root — repo,
pin transport, gate set, companions, diamonds — and moved by one shared reusable
workflow, `.github/workflows/satellite-bump.yml`, called locally by a thin
`.github/workflows/bump-<name>.yml` per satellite. Before this, three satellites had a
hand-written ~600-line lane each and eight had none, which is how dravr-cageux shipped
five releases in a week against a pin nobody moved (carnet#419).

- **Adding a satellite**: a stanza in `satellites.toml`, a `bump-<name>.yml` caller
  (~30 lines; copy `bump-commere.yml`), and a `notify-platform-release.yml` producer
  half in the satellite repo. `DRAVR_PLATFORM_DISPATCH_TOKEN` is an org secret with
  visibility `all`, but the org is on GitHub's free plan, where an org secret reaches **public**
  repos only. A private satellite needs a repo-level secret of the same name (enforme carries
  one; canot's v0.4.29 announce ran with an empty token and exited 4 — carnet#472), or its
  release never fires the lane and the pin moves only on the weekly cron.
- **The version-shaped logic is `scripts/ci/satellite-pin.sh`**, tested by
  `satellite-pin.test.sh` against a fixture per pin shape. Change the rewriter there,
  not in YAML, and add the fixture — two rules it must keep: substitute **in place**
  (architectural-validation.sh:852 needs canot's exact key order) and verify
  **positively** (a negative check passes when the sed matched nothing).
- **Never hardcode a manifest path or an alias list.** Pin sites are discovered by
  grep and aliases resolved from `package = "..."`. A hardcoded path is what killed
  the photograveur lane for weeks (`89155c33e`); a hardcoded alias list is carnet#323.
- **Lane ownership is derived, never declared** — `satellite-pin.sh lane <name>` greps
  the callers. A declared table claimed tronc's chain moved dravr-stripe while nothing
  did, and suppressed its drift on that basis.
- `scripts/ci/check-satellite-drift.sh` reads the same file and exits 2 when a pin has
  no stanza or a stanza has no pin, so the declaration cannot drift from the tree.
- **Not on this spine:** `contremaitre-bump.yml` (hourly, rev pin, pushes direct to
  main) and `tronc-bump.yml` (producer-hosted in dravr-tronc, `@main`, serves eleven
  consumers). Both are deliberate; see their stanza comments.
