# Conversation-Turn Observability

A **conversation turn** is one user utterance (from the chat route, a channel
webhook, or a direct MCP tool call) plus the full chain of LLM and tool calls
the pipeline makes to produce the reply. Every call in that chain shares one
`ConversationTurnId` so operators can retrieve the full trace by a single key.

This page is the on-call reference for the turn id lifecycle, the
`/internal/conversation-turn/{turn_id}` endpoint, and the shape of rows in
`llm_usage`.

## When to use this

- A user reports a slow or wrong reply and you have the turn id from their
  channel log or the analytics dashboard.
- You want to compare what the LLM actually saw across multiple provider calls
  inside one turn (e.g., did the tool loop call `get_activities` before
  generating the summary?).
- You're debugging a cost spike and need to know how many LLM calls one turn
  actually triggered.
- You're writing a Botium-style end-to-end test that needs to assert a full
  trace by id.

## Where the turn id is generated

Exactly one place per entry point. Downstream code **must not** regenerate it —
the `scripts/ci/check-conversation-turn-id.sh` gate enforces this.

| Entry point | Boundary file | How the id enters |
|---|---|---|
| Web chat (`POST /api/chat`) | `routes/chat.rs` | `ConversationTurnId::new()` when the request lands |
| Channel messaging (`POST /api/messaging/{channel}`) | `dravr-canot` webhook adapter → `IncomingMessage.turn_id` | Canot stamps the id at its webhook boundary; pierre-platform adopts it via `From<CanotTurnId>` in `messaging_ingress::persist_single_message` |
| MCP tool calls | `mcp/tool_handlers.rs` | `ConversationTurnId::new()` at handler entry |

## The `/internal/conversation-turn/{turn_id}` endpoint

Admin-only HTTP endpoint that returns every `llm_usage` row for the given turn,
plus an aggregate summary.

### Auth and scoping

- Requires a JWT with an `admin` or `owner` role on the caller's tenant.
- Returns **403** for regular users.
- Returns **403** if the caller's tenant does not own the turn. Cross-tenant
  lookups are never allowed.

### Path parameter

- `turn_id` is a standard UUID string (36 chars, hyphenated).
- **400** if not a valid UUID.
- **400** if the caller passes the nil UUID
  (`00000000-0000-0000-0000-000000000000`). The nil UUID is the sentinel used
  by the pre-migration backfill — returning every pre-migration row lumped
  together as one "turn" would be misleading, so the handler refuses.

### Response shape

```json
{
  "turn_id": "...",
  "tenant_id": "...",
  "user_id": "...",
  "conversation_id": "...",
  "total_tokens": 366,
  "total_cost_usd": 0.0018,
  "total_latency_ms": 3912,
  "tools_called": ["get_activities", "get_training_load"],
  "llm_calls": [
    {
      "provider": "google",
      "model": "gemini-2.0-flash-exp",
      "call_type": "chat",
      "prompt_tokens": 123,
      "completion_tokens": 24,
      "execution_time_ms": 501,
      "created_at": "..."
    },
    ...
  ]
}
```

- `llm_calls` contains one entry per **real** LLM provider call, in the order
  they were made. It never includes the terminal summary row.
- `tools_called` comes from the summary row (authoritative). If the summary
  row is missing (e.g., the pipeline crashed before writing it), the handler
  falls back to the union of `tools_called` across per-call rows.
- `total_latency_ms` is the end-to-end turn wall-clock from the summary row
  (includes tool execution between LLM calls). If the summary is missing, it
  falls back to the sum of per-call `execution_time_ms`.
- `total_tokens` and `total_cost_usd` aggregate across per-call rows only —
  the summary row has zeroed tokens by design, so it never double-counts.

## Row shapes in `llm_usage`

There are exactly two shapes of row per turn:

### Per-call row — one per LLM provider call

- Written by `services::tool_execution::LlmCallRecorder` inside each of the
  three tool-loop variants (web chat, messaging, headless CLI).
- `call_type` is the flow tag: `"chat"`, `"insight"`, or `"messaging"`.
- Real `prompt_tokens` and `completion_tokens` from the provider.
- `execution_time_ms` is the wall-clock of that single `complete()` call.
- `tools_called` is always `"[]"` on per-call rows.

### Turn-summary row — exactly one per turn

- Written by the chat pipeline (`record_llm_usage` in `routes/chat.rs`,
  `record_messaging_llm_usage` in `services/messaging_ingress.rs`, etc.)
  after the pipeline completes.
- `call_type` is the constant `TURN_SUMMARY_CALL_TYPE` (`"turn_summary"`).
- Tokens are **zero** by design — the summary row is not a real provider
  call, so counting its tokens would double-count.
- `tools_called` is the ordered list of MCP tools the tool loop invoked.
- `execution_time_ms` is the end-to-end turn time.

Aggregate analytics queries in `llm_usage.rs` and
`plugins/postgres/usage.rs` filter `WHERE call_type != 'turn_summary'` so
token sums and call counts reflect real LLM activity only.

## Schema

Two migrations — one per backend — added in 2026-04-20:

- `migrations/20260420000001_llm_usage_conversation_turn_id.sql` (SQLite)
- `migrations_pg/20260420000001_llm_usage_conversation_turn_id.sql` (PostgreSQL)

Columns added:

- `turn_id TEXT NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000'` —
  nil UUID sentinel for pre-migration rows.
- `tools_called TEXT NOT NULL DEFAULT '[]'` — JSON array, scoped to the
  summary row.
- Index on `turn_id` for the endpoint's `find_llm_usage_by_turn_id` lookup.

## Finding a turn id from a support ticket

1. From an on-call alert or a user report, grab the **conversation id** (e.g.
   Slack channel+ts, Telegram chat+message id, or web chat `conversation_id`).
2. Query the `messaging_messages` table (or the web `messages` table) for the
   row matching the conversation id and timestamp.
3. The `llm_usage` rows join on `conversation_id + user_id + tenant_id + a
   narrow time window`. There is no direct join column yet — if the time
   window has multiple turns, grep `logs` for the `turn_id` field on the
   pipeline span (canot's `webhook_received_span` and pierre-platform's
   `chat_pipeline::run` span both carry it).
4. Use the admin JWT from `logs/admin-token.txt` (see
   `bin/setup-db-with-seeds-and-oauth-and-start-servers.sh`) to hit
   `GET /internal/conversation-turn/{turn_id}`.

## Forbidden patterns (enforced)

`scripts/ci/check-conversation-turn-id.sh` blocks:

- `turn_id: Uuid::new_v4()` anywhere — use `ConversationTurnId::new()`.
- `ConversationTurnId::new()` inside functions that already take a
  `DispatchResult`, `TurnInput`, or `PendingDispatch` parameter. Those
  functions are downstream of the boundary; regenerating the id defeats the
  per-turn trace.

An allow-list of boundary files sits at the top of the script. Adding a file
there is the same as claiming a new inbound boundary exists; that always
deserves code review.

## Related

- The `ConversationTurnId` newtype lives in
  `crates/pierre-core/src/models/conversation.rs`, with `From<Uuid>` and
  `From<dravr_canot::turn::ConversationTurnId>` bridges.
- `dravr-canot` stamps the id on `IncomingMessage.turn_id`, `OutgoingMessage`,
  `DeliveryReceipt`, and `OutboundQueueEntry` so the full send path is keyed.
- `dravr-embacle`'s `ChatRequest.turn_id` is unused today — the platform
  records per-call `llm_usage` rows at the `tool_execution` layer instead.
