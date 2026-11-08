// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The theory behind every admin screen: the question it answers, how its numbers are computed, since when, and its commits
// ABOUTME: One entry per admin tab id, shown by ScreenTheoryPanel at the top of the screen (carnet#1441)

/**
 * How these entries stay true.
 *
 * - `__tests__/screenTheory.test.ts` fails when an admin tab has no entry, when
 *   an entry names a tab the console no longer draws, and when a cited commit
 *   does not resolve in this repository, is not an ancestor of HEAD, or has a
 *   date or subject other than `git show -s --format=%as%x09%s` prints. It runs
 *   git and fails, never skips, when git cannot answer.
 * - A change to what a screen computes (a query, a filter, a window, a formula,
 *   who may write) updates that screen's `computation` in the same commit, and
 *   once landed on main, its commit joins `commits`. The test cannot read prose,
 *   so this half is the reviewer's to hold.
 * - `since` is the first commit that added the screen's component
 *   (`git log --follow --diff-filter=A`). The repository's root commit
 *   1b33079e0 (2026-05-05) is an import, not an origin, so a screen already in
 *   it reads "before 2026-05-05" and that commit is never cited.
 * - Only commits on main are cited: a branch commit disappears when the branch
 *   is squashed.
 *
 * Admin screens are English-only, so the copy is plain text, not `t()` keys.
 */

export interface TheoryCommit {
  /** Short SHA, at least 9 characters, of a commit on main. */
  sha: string;
  /** Author date, `git show -s --format=%as`. */
  date: string;
  /** Subject line, exactly as `git show -s --format=%s` prints it. */
  subject: string;
}

export interface ScreenTheory {
  /** The question an operator opens this screen to answer. */
  question: string;
  /** Why the screen exists and the idea its numbers rest on. */
  theory: string;
  /** How each figure, list or write on the screen is computed, one line each. */
  computation: readonly string[];
  /** The date of the commit that introduced the screen, or "before 2026-05-05". */
  since: string;
  /** The commits that created the screen or changed what it computes, oldest first. */
  commits: readonly TheoryCommit[];
}

/** Every commit an entry cites, once, so a long subject is written in one place. */
const COMMIT = {
  "304c4ca17": {
    sha: "304c4ca17",
    date: "2026-11-08",
    subject:
      "feat(admin): admin console tells the truth, in one scope, with theory",
  },
  "68d0469ef": {
    sha: "68d0469ef",
    date: "2026-08-26",
    subject:
      "feat(admin): the pre-approval list had no transport off the laptop",
  },
  aa52f26fb: {
    sha: "aa52f26fb",
    date: "2026-10-22",
    subject:
      "feat(signup): signup codes an admin or coach hands out approve users",
  },
  c3e3bcf93: {
    sha: "c3e3bcf93",
    date: "2026-11-03",
    subject: "feat(admin): new-user cohort view and daily digest",
  },
  "2d3619d11": {
    sha: "2d3619d11",
    date: "2026-05-12",
    subject:
      "fix(admin): nav-audit fixes (16 issues) + system-coach use_count + verdict coach_id backfill Backend: avg_execution_time_ms wired into UsageDataPoint via SQLite+PG aggregation (Analytics 'Avg Response Time' was hardcoded 0); claim_verdicts.coach_id now flows from chat_conversations on insert + backfill migration 20260507000002 pulls coach_id forward from the conversation or user default for legacy rows so Myth Busting / Coach Grades / Engagement attribution work; messaging_ingress resolves user.default_coach_id and bumps coach_assignments.use_count via record_usage on freshly-forged conversations (including system coaches from non-pinning tenants — record_use_count was silently no-op'd before); web chat route does the same after create; claim_extractor drops fragments under 6 words so the verifier stops grading 'Yes, but with restraint.' as unsupported; activate_coach allowed for system coaches from non-pinning tenants. Frontend: EngagementTab DAU/WAU/MAU cumulative (DAU ⊆ WAU ⊆ MAU), 'Tokens Used' → 'Prompt Size'; CoachStoreManagement Published/Total; MemoryExtractionMonitor Hamilton largest-remainder so percentages sum to 100; HarnessConfigTab explicit max attrs (Window 2M, Summarize/Sliding 100, Max response 1M); CoachNotesAuditTab help references coach_note_add; AdminSettings collapses auto-approve description; NotificationsPanel inner h1→h2; useUnreadCount poll 60s+focus-refetch → 120s no-focus-refetch; GroupCard break-words + title-tooltip; Dashboard mirrors activeTab → location.hash; SystemPromptsTab status spells out system / coach / contremaitre / compiled-in counts. Locked by frontend/e2e/admin-nav-audit-fixes.spec.ts + coaches_database_test.rs.",
  },
  "68799ca18": {
    sha: "68799ca18",
    date: "2026-09-06",
    subject: "feat(rename): the persona an athlete talks to is an agent",
  },
  cc5a5b153: {
    sha: "cc5a5b153",
    date: "2026-10-21",
    subject:
      "feat(agents): one team-membership rule; onboarding lead is selected",
  },
  "86901370d": {
    sha: "86901370d",
    date: "2026-11-04",
    subject:
      "fix(web): one theme on every admin screen, and text never over text",
  },
  "46ae96122": {
    sha: "46ae96122",
    date: "2026-07-16",
    subject:
      "feat(admin): per-user tool allow/deny + operator tier/plan controls on CLI and web New user_tool_overrides overlay (UserOverride rung: above plan/tenant, below global-disable) gates tools/list+tools/call per user; shared admin_ops tier/plan/tool fns power pierre-cli (user set-tier/clear-tier, tenant set-plan, tool enable/disable/reset/list), new cookie /api/admin routes, and the React admin panel (User Tools tab, tier editor, tenant plan card); 10 integration + PG-smoke + 7 component tests.",
  },
  a74eebf81: {
    sha: "a74eebf81",
    date: "2026-07-16",
    subject:
      "feat(admin): tenant-plan GET + wire-level per-user override e2e + admin-panel sweep fixes GET /api/admin/tenants/{id}/plan preselects TenantPlanCard's current plan; mcp_tools_list_user_override_e2e_test pins the dispatcher glue on the real wire; Playwright admin-user-tools spec (3 green); sweep fixes: per-user panel hides role-gated admin tools, tier editor normalizes display-case tier so Save can't silently downgrade.",
  },
  c888b8d2f: {
    sha: "c888b8d2f",
    date: "2026-11-06",
    subject: "fix(admin): one token screen, operators lose personal panes",
  },
  "49257c7d0": {
    sha: "49257c7d0",
    date: "2026-09-28",
    subject:
      "fix: September review sweep, second batch (auth, admin, sats, dead code)",
  },
  f6fcfbe91: {
    sha: "f6fcfbe91",
    date: "2026-05-19",
    subject:
      "feat(features): runtime feature flags — tenant defaults + per-user overrides New tenant_feature_defaults + user_feature_overrides tables (SQLite+PG); FeatureKey enum (api_tokens, billing_header, both default off); FeatureFlagsRepository wired into RepositoryRegistry; service layer enforces super-admin OR tenant-membership; routes: GET /api/me/features plus GET|PUT|DELETE /api/admin/{tenants|users}/:id/features[/:key]; resolution = user override > tenant default > compile default; useFeatureFlags hook gates API Tokens tab + Billing Current Plan card; sidebar Billing→Usage; admin UI panels in AdminSettings + UserDetailDrawer; 8 backend integration tests + UserSettings test mock.",
  },
  "64f8c89b1": {
    sha: "64f8c89b1",
    date: "2026-10-17",
    subject:
      "feat(signup): guard self-serve sign-up before it can activate itself",
  },
  "3e280bee1": {
    sha: "3e280bee1",
    date: "2026-06-22",
    subject:
      'docs: name claim-verification stages, retire "Tier 5.5"/"Layer 2.5" labels The fractional tier/layer numbers were cargo-cult — the code already uses semantic names (VerdictLayer::{Rhetoric,Deterministic,Personalized,Evidence,Consistency,Judge}); replace the numeric labels in comments/docs with stage names, drop "Tier 5.5" from the LLM-facing verify_claim tool description (+matching SDK type), add a note disambiguating the 3 overloaded "tier" ladders, keep one canonical heading where the ladder is defined.',
  },
  cc4ca386a: {
    sha: "cc4ca386a",
    date: "2026-09-21",
    subject: "fix: webhooks sync for real, verdict triage, iOS bundle warm-up",
  },
  "9090380e9": {
    sha: "9090380e9",
    date: "2026-10-12",
    subject:
      "feat: eight carnet issues — coach review, claim checks, invites, push",
  },
  c8c1652aa: {
    sha: "c8c1652aa",
    date: "2026-05-06",
    subject:
      "feat(harness): wire admin harness config into chat dispatch path HarnessConfigRegistry loads system_settings.harness_config at startup, pre-builds CompactionConfig + Arc<TextGuardrails> projections, and is swapped live by PUT /admin/settings/harness so saves apply without restart (replaces hardcoded ::default() calls in compaction.rs and guardrails.rs stages); 6 new integration tests; bonus: claim_verdicts admin filter UUID-validates coach_id.",
  },
  afe018126: {
    sha: "afe018126",
    date: "2026-05-18",
    subject:
      'fix(coach): 6-bug pack — block-on-refresh import, locale-aware guardrails, prompt rules Bug 1 imports tokio::time::timeout (CI failed clippy::absolute_paths, deploy correctly skipped). Bug 6 reshapes TextGuardrails to per-locale trigger lists + disclaimer text with Unicode word-boundary regex so French "pain" (bread) no longer fires the English medical disclaimer; schema_version 2; ships en/fr/es/de/pt defaults; admin UI now edits per-locale. Bugs 3 + 5 via contremaitre prompt: re-fetch + recompute aggregates on freshness pushback, disambiguate "côté X, combien" between stats vs recommendation. Bugs 2 + 4 auto-fix once this deploys (the 4f9ca4e5 blocking-refresh fix has been blocked from production by Bug 1).',
  },
  "776bf3435": {
    sha: "776bf3435",
    date: "2026-06-12",
    subject:
      "fix(compaction): cap conversation by message count so dense threads can't bypass the token trigger chars/4 estimate under-counts JSON/tool-result content; adds max_messages backstop + budget-driven slide-to-fit.",
  },
  eb43745d7: {
    sha: "eb43745d7",
    date: "2026-08-11",
    subject:
      "feat(guardian): runtime-configurable Guardian and harness surfaces",
  },
  "091feff06": {
    sha: "091feff06",
    date: "2026-08-28",
    subject:
      "fix(compaction): window_tokens names the real context window, not 128k",
  },
  "31eedd624": {
    sha: "31eedd624",
    date: "2026-09-02",
    subject: "feat(memory): a restated fact merges instead of piling up",
  },
  "9b9e97baa": {
    sha: "9b9e97baa",
    date: "2026-09-03",
    subject: "fix(compaction): summarize 12 turns per pass, not 6",
  },
  "8e82d8b3c": {
    sha: "8e82d8b3c",
    date: "2026-11-05",
    subject:
      "fix(guardian): harden the config plane, turn semantics and clients",
  },
  "49556a316": {
    sha: "49556a316",
    date: "2026-09-18",
    subject:
      "fix: eight follow-ups to the Claude App notes — nothing lost to a deploy",
  },
  e5f1b3144: {
    sha: "e5f1b3144",
    date: "2026-10-12",
    subject:
      "feat(llm): each LLM stage has its own model; side calls bill own rows",
  },
  "55bd5df59": {
    sha: "55bd5df59",
    date: "2026-05-07",
    subject:
      "feat(admin): myth-busting promote + followups scheduler + notes suppress Three Outstanding-items shipped: POST /api/admin/myth-busting/promote-topic appends a recurring claim to harness_config.guardrails.blocked_topics + installs the new HarnessConfigRegistry snapshot (idempotent, case-insensitive); services::coach_followup_scheduler ticks every 60s, fetches due_at <= now pending followups via list_due_followups, dispatches a coach_followup_due push and marks delivered_at; coach_notes gains a suppressed BOOL column (SQLite + PG migrations) with admin POST /suppress|/unsuppress route + list_coach_notes filters out suppressed rows so memory recall stops re-injecting flagged content. 15 backend integration tests + frontend Promote/Suppress buttons. Chrome DevTools sweep validated all three end-to-end.",
  },
  "549875a76": {
    sha: "549875a76",
    date: "2026-05-28",
    subject: "fix(evals): run claim-verifier consistency + LLM-judge layers",
  },
  "0e0146eea": {
    sha: "0e0146eea",
    date: "2026-06-05",
    subject:
      "fix(dd): remediate all technical-pitch due-diligence findings (P0-P3 + deferred) Squash of feature/pitch-findings-p0-p2-fixes closing every Appendix A finding: P0 (2) security holes, P1 (11) features built for real, P2 (14) scaffold/quality, P3 tech-debt; plus the goals PostgreSQL migration (set_goal was broken on Cloud Run, caught by the new schema-parity guard) and threshold_estimation wired into export_dossier; sole deliberate keep is the oauth2_auth_codes/authorization_codes table-name divergence (ADR-018); post_process.rs conflict kept both Stage 16a acronym expansion and strict-mode conformance.",
  },
  "1e950d038": {
    sha: "1e950d038",
    date: "2026-10-28",
    subject:
      "fix(coach,i18n,admin): cross-tenant coach review, true corrector, counts",
  },
  "139b0df65": {
    sha: "139b0df65",
    date: "2026-09-07",
    subject: "refactor(api): the routes and commands say agent too",
  },
  bb90f0873: {
    sha: "bb90f0873",
    date: "2026-06-06",
    subject:
      "feat(observability): per-turn tool-usage logs + /admin/tool-usage endpoint + admin panel",
  },
  "432396b66": {
    sha: "432396b66",
    date: "2026-10-14",
    subject:
      "perf(web): Lighthouse pass on the signed-in web app and the phone",
  },
  "78ba74066": {
    sha: "78ba74066",
    date: "2026-07-09",
    subject:
      "fix(security): admin financial endpoints enforce tenant access (CWE-863) handle_get_tenant_usage/invoice call verify_admin_tenant_access; handle_export_billing is super-admin-only; per-user usage + cost-timeseries route through a shared authorize_admin_for_user helper (super-admins stay global, tenant-scoped admins restricted). Closes the cross-tenant financial-data disclosure the Tempest bake-off + Phase-4 audit found. +9 regression tests.",
  },
  "9af6e6490": {
    sha: "9af6e6490",
    date: "2026-09-28",
    subject:
      "feat: provider descriptions, dead repo methods, API keys in Settings",
  },
  "19c01f6c0": {
    sha: "19c01f6c0",
    date: "2026-11-04",
    subject: "feat(admin): an audit trail of every operator write",
  },
  "038493b06": {
    sha: "038493b06",
    date: "2026-05-18",
    subject:
      "fix(auth): align users.is_admin with role across DB + frontend + admin setup Migration normalizes is_admin = (role IN admin/super_admin); UserSettings gates on role (matches Dashboard); admin/setup writes role=Admin so bootstrap row doesn't drift.",
  },
} as const satisfies Record<string, TheoryCommit>;

/** Exported for the staleness test, which checks every cited commit against git. */
export const CITED_COMMITS: readonly TheoryCommit[] = Object.values(COMMIT);

const BEFORE_IMPORT = "before 2026-05-05";

/**
 * The scope rule most screens share since the admin console redesign: a super
 * admin sees every tenant by default and can pick one in the scope bar; a
 * plain admin is pinned to their own tenant and the server refuses any other.
 */
const SCOPE_BAR =
  "Scope: a super admin sees all tenants by default and can pick one tenant in the scope bar; a plain admin always sees their own tenant, and the server refuses a request for any other.";

/** Keyed by dashboard tab id: every tab in ADMIN_ONLY_TABS, plus `settings` as an operator sees it. */
export const SCREEN_THEORY: Readonly<Record<string, ScreenTheory>> = {
  users: {
    question:
      "Who is waiting to get in, who is active or suspended, and who do I approve?",
    theory:
      "Dravr gates sign-up behind approval, so an operator needs one queue of who is waiting and one list of every account with its status. Approving an account is what lets it use the product.",
    computation: [
      "Pending: every user with status pending, newest first, no limit, re-read every 30 seconds.",
      "Active, Suspended and All: the full user list, read 100 per page through the cursor and filtered by status in the browser. Search runs in the browser on email and name.",
      "Neither list filters by tenant: every operator with user-management rights sees every tenant's users.",
      "Approve sets the status, who approved and when, moves the user into the approver's active tenant when there is one, creates an MCP token, and sends the approval email and event. Suspend changes the status only.",
      "New users, Pre-approved, Signup codes and Coach access are separate lists on their own sub-tabs.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["68d0469ef"],
      COMMIT["aa52f26fb"],
      COMMIT["c3e3bcf93"],
      COMMIT["304c4ca17"],
    ],
  },
  agents: {
    question:
      "Which system agents exist, what do they say, and who is assigned to them?",
    theory:
      "System agents are the personas Dravr ships. Each is a prompt plus metadata; editing one changes what every athlete assigned to it talks to, so an edit records a version first and notifies the assigned athletes.",
    computation: [
      "List: agents with is_system set in the session's active tenant, newest first.",
      "Uses: the sum of agent_assignments.use_count over that tenant's members, all time. The single-agent read returns 0, so the detail view after an edit shows 0 uses.",
      "Tokens: an estimate of the system prompt's size, stored when the agent is written.",
      "Assigned users: assignment rows marked as members, limited to this tenant. Unassigning keeps the counters.",
      "An edit saves the title, description, prompt, category, tags and sample prompts. Visibility is set only at creation. A create or edit naming any other field is refused (422), never accepted and dropped.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["2d3619d11"],
      COMMIT["68799ca18"],
      COMMIT["cc5a5b153"],
      COMMIT["86901370d"],
      COMMIT["304c4ca17"],
    ],
  },
  "agent-store": {
    question:
      "What is waiting for store review, and what is published or rejected?",
    theory:
      "An agent reaches the store only through moderation. Each listing moves pending, then published or rejected, and every transition checks the status it starts from so two operators cannot both decide one listing.",
    computation: [
      "Pending, Published and Rejected counts and total installs: one query on store_listings in the active tenant. Installs sum install_count over published listings.",
      "Rejection rate: rejected / (published + rejected) × 100, computed by the server; 0 when nothing has been decided.",
      "Published / Total compares the tenant's published listings with the number of system agents, which lives in a different table.",
      "Review queue: pending listings in the tenant, oldest submission first, 50 at a time. Rejected: newest decision first.",
      "The published list is not limited to the tenant, so it can be longer than the Published count above it.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["2d3619d11"],
      COMMIT["68799ca18"],
      COMMIT["86901370d"],
      COMMIT["304c4ca17"],
    ],
  },
  configuration: {
    question: "Which MCP tools can this tenant use, and why?",
    theory:
      "A tool is available through a ladder of rules, highest first: a global disable, the tenant plan, a tenant override, then the catalogue default. This screen shows where on that ladder each tool lands for the tenant.",
    computation: [
      "The tenant is the signed-in operator's own; there is no tenant picker. A super admin may reach any tenant; another admin only a tenant they belong to.",
      "Status per tool: disabled by PIERRE_DISABLED_TOOLS, else disabled when the plan is below the tool's minimum plan, else the tenant override, else the catalogue default. An unknown tenant is treated as Enterprise.",
      "Summary counts are counted from that same list.",
      "Tenant plan (super admins only): a change writes the tenant's plan and clears its tool cache.",
      "An override is stored with the operator who made it and clears the cache on the instance that served it. The cache lives five minutes, so other instances can serve the old answer until theirs expires.",
    ],
    since: BEFORE_IMPORT,
    commits: [COMMIT["46ae96122"], COMMIT["a74eebf81"], COMMIT["304c4ca17"]],
  },
  "user-tools": {
    question:
      "Which tools can one particular user call, and can I force one on or off for them?",
    theory:
      "A per-user override is the rung above the tenant's rules and below a global disable. It is kept out of the tenant cache on purpose, so a change is never served stale.",
    computation: [
      "User picker: the all-users list, searched in the browser, first 25 matches.",
      "Tool list: the tenant result for the user's first tenant, with the user's overrides laid over it, read fresh on every request. Admin tools are hidden because they are gated by role anyway.",
      "A plain admin may open a user only when they share a tenant.",
      "An override is stored per user and tool, not per tenant, and applies at once to MCP tool calls, the chat tool catalogue and the Guardian.",
    ],
    since: "2026-07-16",
    commits: [
      COMMIT["46ae96122"],
      COMMIT["a74eebf81"],
      COMMIT["c888b8d2f"],
      COMMIT["304c4ca17"],
    ],
  },
  prompts: {
    question:
      "Which prompts is the server running right now, and where did each come from?",
    theory:
      "Prompts are authored in the dravr-contremaitre repository and loaded into an in-memory registry at startup and on sync, with copies compiled into the binary as the fallback. Nothing here is read from the database.",
    computation: [
      'Counts and sources come from the registry: "synced" means loaded from contremaitre, "seed copy" means the compiled-in fallback.',
      "Size is the prompt's length in bytes. Agents are listed by slug with their locales; a missing translation falls back to the default locale, which is fr.",
      "The status bar, system prompts and Sync Now load only for a super admin (configuration rights). Agent prompts are open to any console admin.",
      "An edit commits the file to contremaitre on GitHub together with the manifest hash that announces it, then applies it on the instance that took the request; every other instance picks it up on its sync poll, every 60 seconds by default. A failed commit changes nothing, and content missing a placeholder its prompt requires is refused. Without contremaitre configured (local development) the edit lives only in that process.",
      "Prompts are platform-wide: nothing on this screen is per tenant.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["2d3619d11"],
      COMMIT["68799ca18"],
      COMMIT["49257c7d0"],
      COMMIT["304c4ca17"],
    ],
  },
  "platform-settings": {
    question:
      "Who can get in, how fast, and which features and group rules are on?",
    theory:
      "Sign-up is one global policy read on every sign-up, so a change takes effect with no deploy. Feature flags resolve user override, then tenant default, then the compiled default.",
    computation: [
      "Auto-approval: the AUTO_APPROVE_USERS environment variable wins, then the stored setting, then the default. It is global, and the toggle is locked while the environment variable is set.",
      "Sign-up: open or closed, a daily limit (default 100, 0 to 100,000) and verified-email activation (default off). An unreadable setting falls back to open.",
      "Counted: users created since the later of 24 hours ago and the last reset, whatever created them. Reset stores the current time.",
      "Feature flags: the operator's own tenant. Group creation is written at global scope; when it is read, a tenant-level override and then an environment pin come before that global value.",
      "Auto-approval and sign-up changes are recorded in the audit trail.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["f6fcfbe91"],
      COMMIT["64f8c89b1"],
      COMMIT["aa52f26fb"],
      COMMIT["304c4ca17"],
    ],
  },
  "claim-verdicts": {
    question:
      "What did the claim checker flag in agent replies, and how much of it was noise?",
    theory:
      "After every agent reply, sentences that make a factual claim are judged in stages: rhetoric, population bounds, the athlete's own data, the evidence corpus, consistency, then an LLM judge. This screen lists those verdicts and is where an operator records whether a flag was right.",
    computation: [
      SCOPE_BAR,
      "Verdict list: claim_verdicts filtered by status, category, agent, layer and disposition, newest first, 50 by default and at most 200. The five status counts are counted by the server over every verdict the filters select, the status filter included, not over the page shown.",
      "Detector health: unsupported and contradicted verdicts inside a rolling 7, 30, 90 or 365 day window, by layer and by reason. False-positive rate = false positives / verdicts given a disposition, 0 when none has one.",
      "Calibration (moved here from the retired Eval Harness tab): every verdict per UTC day over a fixed 30 days. Pass rate = supported / (supported + unsupported + contradicted).",
      "A disposition overwrites the previous one with the reason, a note of up to 2,000 characters, who and when. It needs configuration rights.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["549875a76"],
      COMMIT["3e280bee1"],
      COMMIT["eb43745d7"],
      COMMIT["cc4ca386a"],
      COMMIT["9090380e9"],
      COMMIT["86901370d"],
      COMMIT["304c4ca17"],
    ],
  },
  "harness-config": {
    question:
      "How large may a prompt grow before history is compacted, and what happens to a reply before the athlete reads it?",
    theory:
      "One platform-wide document holds the chat pipeline's dispatch settings: conversation compaction, reply text guardrails, the claim judge switch and memory dedup. The warn and emergency fractions (0.0896 and 0.1216 of a 1,000,000-token window) keep the triggers at 89,600 and 121,600 tokens, the counts the old 128,000-token window gave at 70% and 95%.",
    computation: [
      'The document is one row, system_settings harness_config. "persisted" means the row exists; "default" means it does not and the compiled defaults apply. A row that does not parse shows this instance\'s copy with the reason.',
      "Compaction: summarize when the conversation exceeds max_messages or its estimated tokens reach warn; drop the oldest turns with no summary when they reach emergency.",
      "Guardrails: a blocked topic found anywhere in a reply (case-insensitive) replaces the whole reply; a disclaimer trigger word in the reader's locale prepends that locale's disclaimer.",
      "Save: validated, then written by compare-and-set on the last updated_at; a stale save gets a 409 and writes nothing. The instance that saves applies it at once; the others on their settings poll, 30 seconds by default.",
      "Super admins only, with a global-scope token. Per-tenant overrides do not exist. Every save is recorded in the audit trail.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["c8c1652aa"],
      COMMIT["afe018126"],
      COMMIT["776bf3435"],
      COMMIT["eb43745d7"],
      COMMIT["091feff06"],
      COMMIT["31eedd624"],
      COMMIT["9b9e97baa"],
      COMMIT["8e82d8b3c"],
      COMMIT["304c4ca17"],
    ],
  },
  "memory-worker": {
    question:
      "Is the background memory-extraction worker running, failing or falling behind?",
    theory:
      "After each reply a background step extracts facts about the athlete. Its health is measured from its own run ledger, not inferred from the fact store, because athlete approvals and agent tools also write facts.",
    computation: [
      SCOPE_BAR,
      "Runs, over 24 hours and 7 days: runs that persisted facts, found none, or failed; facts persisted; the last finish; the last failure. Run rows are pruned after 30 days.",
      "Backlog: extraction jobs created more than one lease (5 minutes) ago.",
      "Health, first match: Failing when every run in 24 hours failed; Degraded on any failure; Backlog when jobs wait; Healthy with a run in 24 hours; Quiet with runs in 7 days only; Idle otherwise.",
      "Fact store: user_facts total, distinct users, rows updated in 24 hours and 7 days, and by kind, with percentages rounded by largest remainder so they sum to 100. Re-read every 60 seconds.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["2d3619d11"],
      COMMIT["49556a316"],
      COMMIT["e5f1b3144"],
      COMMIT["304c4ca17"],
    ],
  },
  "myth-busting": {
    question:
      "Which claims were athletes recently warned about, from which agents, and in which categories?",
    theory:
      "A claim is a whole sentence of an agent reply, so the same words rarely repeat and an occurrence count shows no pattern. The screen therefore lists recent flagged claims by date, read from the claim checker's verdicts on each load, with no worker of its own.",
    computation: [
      SCOPE_BAR,
      "Scan: the newest 500 claim_verdicts rows. There is no date filter; the window shown runs from the oldest to the newest row scanned.",
      "Warned: contradicted, or unsupported outside training advice, and not marked a false positive.",
      "Two claims are the same when their words match after lowercasing and removing punctuation.",
      "Recent claims: top 10 by latest occurrence. Top agents and top categories: top 10 by warned count; verdicts with no agent are left out of the agent list.",
      "A read beyond the operator's own tenant is recorded in the audit trail. Refreshes every 2 minutes.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["55bd5df59"],
      COMMIT["549875a76"],
      COMMIT["eb43745d7"],
      COMMIT["cc4ca386a"],
      COMMIT["8e82d8b3c"],
      COMMIT["304c4ca17"],
    ],
  },
  "agent-grading": {
    question: "Which agents make the least supported factual claims?",
    theory:
      "Each agent is graded from the claim checker's verdicts on its replies, recomputed on every load. Only verdicts from the LLM judge era count: before eb43745d7 (2026-08-11) an inconclusive claim was recorded as unsupported, so the two periods measure different things. The weights and bands are engineering choices, not a published standard.",
    computation: [
      SCOPE_BAR,
      "Scan: the newest 500 verdicts. Only those from 2026-08-12 00:00 UTC on count; older ones and judge-era verdicts with no agent are reported separately.",
      "Score = (supported + 0.25 × unsupported − contradicted) / (supported + unsupported + contradicted), clamped to 0..1, and 0.5 when nothing is scored.",
      "Not scored: rhetorical, unverifiable, unsupported training advice (which has no corpus to cite) and false positives.",
      "Grade: Provisional under 3 scored verdicts, else A ≥ 0.90, B ≥ 0.75, C ≥ 0.60, D ≥ 0.40, F below. Sorted worst first; Low grades counts D and F.",
      "The grade does not reorder the agent store.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["2d3619d11"],
      COMMIT["0e0146eea"],
      COMMIT["eb43745d7"],
      COMMIT["86901370d"],
      COMMIT["304c4ca17"],
    ],
  },
  activity: {
    question: "What is happening on the platform right now?",
    theory:
      "A live operator feed, re-read every 10 seconds, of LLM calls and conversations as they happen. A failed read is shown as an error, never as a quiet platform.",
    computation: [
      SCOPE_BAR,
      "Recent LLM calls: the newest 20 llm_usage rows, leaving out turn summaries and MCP tool rows. Recent conversations: the newest 20 by last update. The two lists fill independently, so they can cover different spans.",
      "Active Conversations (15m): conversations updated in the last 15 minutes.",
      "Calls, tokens and cost today: llm_usage since UTC midnight with the same exclusions. Cost is today's usage priced at current prices and summed, not an extrapolation.",
      "Search and the type filter work on the 40 rows on screen only.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["e5f1b3144"],
      COMMIT["1e950d038"],
      COMMIT["86901370d"],
      COMMIT["304c4ca17"],
    ],
  },
  engagement: {
    question: "How many users are active, and which agents get used?",
    theory:
      "Activity tiers are cumulative, so a daily user also counts as weekly and monthly: the older separate bins could show more daily than weekly users. Use counts come from agent assignments.",
    computation: [
      SCOPE_BAR,
      "Every figure follows that one scope, read by the server in one request: a tenant's figures are its members' and its own agents', the platform's are every user's and every system agent's.",
      "DAU, WAU and MAU: users in scope whose last_active is within 1, 7 and 30 days; Dormant beyond 30, or never active. last_active is stamped on login and on authenticated requests, at most once per refresh interval.",
      "Agent leaderboard: system agents in scope, ranked by the sum of agent_assignments.use_count of the scope's users (all time), then title; top 20.",
      "Published Agents: store listings published in scope.",
      "Requests: the sum of the Usage Analytics series. Only this figure follows the time range.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["2d3619d11"],
      COMMIT["139b0df65"],
      COMMIT["86901370d"],
      COMMIT["304c4ca17"],
    ],
  },
  connections: {
    question:
      "Which service tokens exist for scripts, CI and pierre-cli, and which do I create, rotate or revoke?",
    theory:
      "Service tokens and admin tokens were one list on two screens, so they are one screen now (carnet#1444): a bookmark of the old Admin Tokens tab opens this one. The token itself is shown once; only its hash and prefix are kept.",
    computation: [
      "List: every admin_tokens row, newest first, with no tenant filter. Super-admin tokens are hidden from anyone who is not a super admin. Active and Inactive are counted in the browser.",
      "Requests and Last used: every request the token authenticates adds one to its row and stamps the time and address, so they count from the token's creation.",
      "Create: only a super admin can create a super-admin token, and anyone else can grant only permissions they hold. 0 days means no expiry.",
      "Revoke deactivates the token, effective on its next request. Rotate deactivates it and issues a new one with the same name, scope and super-admin flag, valid 365 days by default.",
      "Connected Apps lists the A2A clients the signed-in operator registered.",
    ],
    since: BEFORE_IMPORT,
    commits: [COMMIT["c888b8d2f"], COMMIT["304c4ca17"]],
  },
  analytics: {
    question:
      "How much is the platform used, how fast does it answer, and what do LLM calls cost?",
    theory:
      "Three panels computed live from llm_usage on every load, with no rollup table and no job. llm_usage records completed calls only, so the screen reports no error rate.",
    computation: [
      SCOPE_BAR,
      "Time range: 7, 14, 30 or 90 days, 7 by default, remembered in the browser.",
      "Usage: calls per UTC calendar day, leaving out turn summaries and MCP tool rows; average response time weighted by calls.",
      "LLM consumption: grouped by provider, model and call type over a rolling now minus N days, priced at current prices; daily cost splits the total across days in proportion to each day's tokens.",
      "Tool usage: llm_usage rows with tools called, all call types; invocations, distinct turns and average latency per tool, where a call's latency is credited to every tool in it.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["2d3619d11"],
      COMMIT["bb90f0873"],
      COMMIT["e5f1b3144"],
      COMMIT["432396b66"],
      COMMIT["86901370d"],
      COMMIT["304c4ca17"],
    ],
  },
  billing: {
    question: "Can I download LLM usage rows to reconcile an invoice?",
    theory:
      "The tab exists only in a build made with VITE_BILLING_ENABLED=true, off by default because billing is outside the first release. The export is platform-wide, so the server allows it to super admins only.",
    computation: [
      "Export: the newest N llm_usage rows made in the chosen month (UTC, from the 1st 00:00 to the next month's 1st), across every tenant, leaving out turn summaries and MCP tool rows, N from 1 to 10,000 (default 1,000), as CSV.",
      "A month that is not YYYY-MM is refused. A month with more than N calls exports only its newest N.",
      "The screen writes nothing.",
    ],
    since: BEFORE_IMPORT,
    commits: [COMMIT["78ba74066"], COMMIT["304c4ca17"]],
  },
  "impersonation-log": {
    question:
      "When did a super admin act as another user, as whom, for how long, and why?",
    theory:
      "Impersonation lets a super admin see the product as a user sees it. Every session is recorded so it can be accounted for afterwards.",
    computation: [
      "List: every impersonation session, active and ended, every tenant, newest first by start time, 100 per page. Load older resumes after the last session shown by start time and id, so sessions that started in the same instant are never skipped.",
      "Duration: end, or now while active, minus start, in minutes with a minimum of 1. A deleted account shows as Unknown user.",
      "Details reads one session for its reason and end time. Starting a session ends the impersonator's active one, and refuses yourself or another super admin.",
      "Super admins only, checked again by the server.",
    ],
    since: "2026-09-28",
    commits: [COMMIT["9af6e6490"], COMMIT["304c4ca17"]],
  },
  audit: {
    question:
      "What did operators change, through the console, pierre-cli or a service token, and what was refused?",
    theory:
      'Every operator write is recorded, so "who deleted that account?" has an answer that is not someone\'s memory. Request bodies are never read, because they carry secrets.',
    computation: [
      "A layer around every admin router records each non-GET request. Outcome from the HTTP status: 2xx and 3xx succeeded, 4xx refused, 5xx failed. Source from the token: console, pierre-cli or a service token.",
      "Scope: a super admin sees every tenant; another admin sees their session's active tenant only. Needs user-management rights.",
      "Newest first by time then id, 50 per page (at most 200). Load older resumes after the last event shown by its time and id, so events recorded in the same instant across a page boundary are all shown.",
      "Filters: outcome and action prefix. A deleted operator's events stay, without who did them.",
    ],
    since: "2026-11-04",
    commits: [COMMIT["19c01f6c0"], COMMIT["304c4ca17"]],
  },
  settings: {
    question: "What can I change about my own operator account?",
    theory:
      "An operator's settings are their own account, not an athlete's, so the athlete panes (providers, gear, messaging, coaching, notifications, memory, privacy, about) are hidden for operators.",
    computation: [
      "Profile: display name, theme and language. The email is shown and cannot be changed here.",
      "Account: status, role and member since; change password; the MCP apps connected to the account, each revocable; sign out.",
      "Tokens and API keys appear only when the api_tokens feature flag is on for the operator or their tenant, and manage the operator's own.",
    ],
    since: BEFORE_IMPORT,
    commits: [
      COMMIT["038493b06"],
      COMMIT["f6fcfbe91"],
      COMMIT["c888b8d2f"],
      COMMIT["304c4ca17"],
    ],
  },
};
