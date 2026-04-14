# Sports Science Evidence Corpus — In-Tree Fallback

**⚠️ READ ME BEFORE EDITING ANY FILE IN THIS DIRECTORY**

This directory holds the **compile-time fallback** for the Tier 5.5 bullshit
detector's evidence corpus. It is **not the source of truth**.

## Two-source pattern

| Concern | In-tree (this directory) | `dravr-contremaitre` |
|---|---|---|
| **Role** | Compile-time fallback | Runtime source of truth |
| **Loaded when** | Always, via `include_str!` in `crates/pierre-server/src/services/claim_verification.rs` | On startup via GitHub sync, on push via webhook |
| **Edited by** | Rust developers (rarely) | Content team + admins (commonly) |
| **Update cadence** | Only when the pattern changes | Per new research |
| **Precedence** | Used when contremaitre is unreachable or the registry is empty | Wins when present |

This matches the pattern used by `pierre-llm/src/prompts/*.md` for system
prompts and coach personas: the in-tree files exist so the server boots and
functions correctly even when GitHub is unavailable. Once `EvidenceRegistry`
lands (Phase A follow-up), `claim_verification.rs::corpus()` will prefer
the registry and fall back to `EMBEDDED_PROPOSITIONS` only when the registry
is empty.

## File format

One proposition per `.md` file, with YAML frontmatter:

```markdown
---
id: doi:10.1136/bjsports-2017-097608
category: nutrition
strength: strong
citation: Morton et al. 2018 BJSM meta-analysis
---

Protein intake of 1.6 to 2.2 g per kg body weight per day maximizes muscle protein synthesis in resistance-trained adults.
```

### Frontmatter fields

| Field | Type | Description |
|---|---|---|
| `id` | string | Stable identifier — DOI (`doi:...`), PMID (`pmid:...`), or ISSN position stand (`issn:...`). Never reuse. |
| `category` | enum | One of: `physiological`, `training_prescription`, `nutrition`, `recovery`, `supplement`, `injury_rehab`. |
| `strength` | enum | One of: `strong` (peer-reviewed meta / position stand), `mixed` (single RCT / conflicting results), `weak` (observational only). |
| `citation` | string | Short human-readable cite for the UI chip (e.g., "Morton et al. 2018 BJSM meta-analysis"). |

### Body

Single paragraph, one atomic factual proposition. No lists, no sub-claims,
no hedging language ("may", "might", "could"). The rhetoric detector will
pick those up as non-propositional and skip them.

## Directory layout

One subdirectory per category (matches `ClaimCategory::as_str()`):

```
sports_science/
├── nutrition/
├── supplement/
├── physiological/
├── training_prescription/
├── recovery/
└── injury_rehab/
```

Filenames are `{first-author}-{year}-{short-topic}.md` — kebab-case,
lowercase, no spaces. The filename doesn't affect parsing but makes git
diffs readable.

## Adding a new proposition

1. Pick the category.
2. Create a new `.md` file under `sports_science/{category}/` with the
   filename pattern above.
3. Fill in the frontmatter and body.
4. **Also add an `include_str!()` entry** to `EMBEDDED_PROPOSITIONS` in
   `crates/pierre-server/src/services/claim_verification.rs`. The array
   is the authoritative enumeration — the loader does not walk the
   filesystem at runtime (it can't, the files are embedded at build time).
5. Run `cargo test -p pierre-evals --test bullshit_detector_test` and
   `cargo test -p pierre_mcp_server --test claim_verification_service_test`
   to confirm the parser accepts it.
6. **After `dravr-contremaitre` integration lands (Phase A follow-up):**
   also commit the same file to `dravr-contremaitre` at
   `evidence/sports_science/{category}/{same-filename}.md`. Until then,
   this directory is the only source.

## Updating an existing proposition

1. Edit the `.md` file in-place. The `id` field must not change — it is
   the stable key used for deduplication.
2. If the evidence strength changed (e.g., a weak claim was upgraded by
   a new meta-analysis), update the `strength:` frontmatter and the
   `citation:` string.
3. After contremaitre integration: push the same change to
   `dravr-contremaitre` so the runtime registry picks it up via webhook.

## Retiring a proposition

1. **Do not delete the file.** Instead, add a `superseded_by:` frontmatter
   field pointing to the new proposition's `id`, and leave the old file in
   place for provenance.
2. Remove the `include_str!()` entry from `EMBEDDED_PROPOSITIONS` if you
   want the server to stop returning it (not yet supported — the loader
   currently ignores `superseded_by`; that will be wired up in Phase D when
   proactive myth-busting lands).

## Why the `EMBEDDED_PROPOSITIONS` array is manual

Rust's `include_str!()` is evaluated at compile time, so we can't walk the
`fixtures/` directory at build time without either:
- A `build.rs` that generates Rust source code (more build complexity), or
- A third-party crate like `include_dir` (new dependency, not on the
  allowlist), or
- The manual enumeration we use today (verbose but explicit, no new deps).

The manual enumeration is the right tradeoff for Phase A: it makes it
impossible to add a file without also wiring it into the loader, which
means the drift between "files on disk" and "files in the corpus" is
always zero.

## Sync with dravr-contremaitre

**Before Phase A follow-up (current state):** no sync, the in-tree files
are the only source. `dravr-contremaitre` has no `evidence/` directory yet.

**After Phase A follow-up:** bidirectional reference but one-way authority.
- Source of truth: `dravr-contremaitre/evidence/sports_science/**/*.md`
- Fallback: this directory + `EMBEDDED_PROPOSITIONS` array
- Drift policy: when content team updates a proposition in contremaitre,
  a Rust developer should port the same change here on the next Pierre
  release to keep the fallback current. This is a manual "catch up"
  process, not an automated sync — we don't want the build to reach out
  to GitHub. Drift of up to one release cycle (typically 1-2 weeks) is
  acceptable for fallback data.

Anything older than a release cycle indicates either:
- A stale proposition in the fallback → update this directory
- Or a new proposition that hasn't been backported → add to this directory

## Related files

- `crates/pierre-evals/src/evidence_retriever.rs` — `EvidenceCorpus`,
  `EvidenceRecord`, `from_markdown_files()`, `from_markdown()`,
  `parse_markdown_record()`.
- `crates/pierre-server/src/services/claim_verification.rs` —
  `EMBEDDED_PROPOSITIONS`, `corpus()` singleton, dispatch wiring.
- `crates/pierre-server/tests/claim_verification_service_test.rs` —
  verifies the embedded corpus parses and returns at least one nutrition
  verdict.
- `crates/pierre-evals/tests/bullshit_detector_test.rs` — end-to-end
  detector pipeline tests using inline sample corpora via
  `EvidenceCorpus::from_jsonl()` (the JSONL helper is retained for tests
  only because inline markdown literals are noisy — production callers
  must use `from_markdown_files`).
