// ABOUTME: The chat turn envelope every surface reads — reply blocks, telemetry, and the terminal turn shape
// ABOUTME: Mirrors crates/pierre-server/src/routes/chat/turn_response.rs; shared verbatim by web and mobile

import type { Message } from './api.js';

/**
 * Interactive control attached to a turn, carried inside an `actions` reply
 * block.
 *
 * Frontends render each action as a clickable button; clicking a `postback`
 * re-sends `value` as the user's next message, flowing back through the same
 * dispatcher.
 */
export interface ChatMessageAction {
  /** User-visible button label. */
  label: string;
  /**
   * Action kind. `"postback"` means the frontend should send `value`
   * as the next chat message. `"url"` means open `value` in a browser.
   * Unknown types should be ignored.
   */
  action_type: string;
  /**
   * For `postback`: the text to send as the next user message
   * (e.g. `/coach select <uuid>`). For `url`: the absolute URL.
   */
  value: string;
}

/** One flagged claim, ready to render as a chip. */
export interface ReplyVerdictChip {
  /** The claim's own sentence, verbatim. */
  claim: string;
  /** `true` when the claim violated a deterministic bound. */
  contradicted: boolean;
}

/** A platform notice attached to a turn. */
export type ReplyNotice = {
  kind: 'quota_warning';
  level: 'approaching' | 'burst';
  current: number;
  limit: number;
  resets_at: string;
};

/**
 * One renderable piece of an assistant reply.
 *
 * The server has already decided which pieces this surface gets: it read the
 * surface's render capabilities and produced this ordered list. Clients lay
 * out what they are given and reconstruct nothing — there is no "if
 * activity_list is present, draw a panel" sniffing left to do.
 */
export type ReplyBlock =
  | { type: 'prose'; text: string }
  /** A chart or table, already resolved into positioned primitives. */
  | { type: 'scene'; scene_blocks: string }
  /** A chart to fetch as an image. */
  | { type: 'scene_image'; url: string; mime_type: string; caption?: string }
  /** A schema-validated workout plan, rendered as a card. */
  | { type: 'workout_plan'; plan: unknown }
  /** The athlete's activities, rendered as their own panel. */
  | { type: 'activity_list'; text: string }
  /** Claim-verification results attached to the reply. */
  | { type: 'verdicts'; chips: ReplyVerdictChip[] }
  /** Controls the athlete can press. */
  | { type: 'actions'; title?: string; actions: ChatMessageAction[] }
  /** A provider connection needs re-authorizing. */
  | {
      type: 'reconnect';
      provider: string;
      display_name: string;
      url: string;
      text: string;
    }
  /** A platform notice about the turn. */
  | { type: 'notice'; notice: ReplyNotice };

/** Cost and provenance facts about a turn. Not for rendering. */
export interface TurnTelemetry {
  model: string;
  provider_name: string;
  tool_calls_count: number;
  tools_called: string[];
  execution_time_ms: number;
}

/** The assistant side of a turn. */
export interface AssistantTurn {
  /** The persisted assistant message — the durable transcript row. */
  message: Message;
  /** What to render, in order. */
  blocks: ReplyBlock[];
  /**
   * Finish reason. `"command"` marks a turn a slash-command handler answered
   * rather than the LLM, so a client can skip LLM-specific treatments.
   */
  finish_reason?: string;
}

/**
 * One completed chat turn, exactly as the wire carries it.
 *
 * The terminal document of a turn on every in-app surface: the whole body of
 * a single-JSON answer, and the payload of the `done` frame when the same
 * turn is delivered progressively. Both come out of one server-side egress,
 * so a client never has two shapes to reconcile.
 */
export interface TurnEnvelope {
  /** Correlation id matching the server's log lines for this turn. */
  turn_id: string;
  user_message: Message;
  assistant: AssistantTurn;
  conversation_updated_at: string;
  /**
   * The conversation the athlete is now on, when the turn moved them.
   *
   * Absent on every ordinary turn. `/reset` sets it: the thread this turn was
   * posted to is archived and the next one belongs here, so a client that
   * reads back an id different from the one it posted to opens that thread.
   */
  rotated_to_conversation_id?: string;
  telemetry: TurnTelemetry;
}

/**
 * Something the turn is doing, observed while it was still running.
 *
 * Each event is a snapshot of that activity's latest known state, so a
 * consumer can either accumulate by `id` or simply render the most recent one
 * it received. Mirrors `pierre_services::chat_stream::TurnProgress`.
 */
export interface TurnProgress {
  /**
   * Whether this is a pipeline stage the turn passes through, or a tool the
   * model asked for. The two read differently to an athlete — a stage says
   * what the coach is doing, a tool names what it is looking at.
   */
  kind: 'stage' | 'tool';
  /** Stable id: the stage's name, or the tool call's protocol id. */
  id: string;
  /** What to name in a status line — the stage name, or the tool's title. */
  title: string;
  /**
   * Latest state, in the producer's own vocabulary: `"started"` /
   * `"finished"` for a stage, and the ACP call state (`"Pending"`,
   * `"InProgress"`, `"Completed"`, …) for a tool.
   */
  status: string;
}
