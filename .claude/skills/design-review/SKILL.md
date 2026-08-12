---
name: design-review
description: Review UI components for design system compliance, accessibility, and visual consistency
user-invocable: true
---

# Design Review

Review UI changes against **`frontend/DESIGN.md`** — the Boreal design system,
Product Tier. DESIGN.md is the source of truth; this skill is how you check
work against it.

## Order of operations

The machine checks run first because they are exhaustive and free. Your judgment
is for what they cannot see.

### 1. Run the gates (don't hand-roll their greps)

```bash
./scripts/ci/design-system-validation.sh      # tokens, primitives, ratchets
cd frontend && bun run lint                   # bans raw <select>/<textarea>
```

Everything these cover — stock Tailwind palette instead of §2 tokens, raw form
controls outside `components/ui/`, retired `.input-dark`/`.select-dark`, missing
token mirrors — is already enforced. If they pass, do not re-audit it by grep.
If a ratchet fell, lower the baseline in the script as instructed.

### 2. LOOK AT IT

**A design review without a rendered screen is a lint run.** The defects that
matter most — three form languages stacked in one card, a stock amber louder
than the primary CTA, a control that collides with the sidebar — are invisible
in a diff and obvious in a screenshot.

```bash
./bin/start-server.sh          # backend on 8081 (see setup-server skill for seeds)
cd frontend && bun run dev     # vite on 5173
```

Then drive the real app with the `chrome-devtools` MCP tools: `navigate_page`,
`take_screenshot`, `resize_page` (test 1440 and 390 wide), `emulate` for dark
and light. Screenshot **every screen the diff touches** and read the image.

For mobile changes, do the same via the `ios-simulator` or `mirroir` MCP tools.

### 3. Judge what only a human eye catches

With the screenshots in front of you:

- **One language per surface.** Do all fields in a card share label casing,
  field chrome, and help-text placement? (DESIGN.md §5 — this is the failure
  mode the primitives were built to end.)
- **Visual hierarchy.** Is the loudest element on the page the one that should
  be? A saturated accent on a metadata chip outshouting the sole CTA is a bug.
- **Rhythm.** Do gaps come from the §6 scale, or are they ad hoc?
- **Both themes.** Boreal ships light and dark; check the change in both.
- **Empty, loading, error, long-content.** Screenshot at least the empty and
  overflow cases — truncation and layout collapse only show up there.

### 4. Accessibility

`frontend/e2e/accessibility/` runs in pre-push and CI, so trust it for icon
labelling and 44x44 touch targets. Add a spec there for anything new it does
not cover. Verify by eye: focus rings visible on every interactive element
(`.focus-ring`, §5), and text contrast ≥ 4.5:1 against its actual background.

## Output

Report findings as `file:line — what's wrong — which DESIGN.md section it
violates`, then the fix. Attach or describe the screenshot for anything visual;
a visual claim without a rendered screen behind it is a guess.

End with: **PASS** / **NEEDS WORK (n issues)**.

## When you find drift the gates missed

That is a gap in the gates, not just in the code. Say so, and propose the rule
that would have caught it — a `no-restricted-syntax` selector for anything
element-shaped, a check in `scripts/ci/design-system-validation.sh` for
anything a text scan can see. Enforcement is what keeps the system from
drifting back; a finding that only lives in a review comment will recur.
