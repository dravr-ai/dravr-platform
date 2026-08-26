// ABOUTME: Auto-generated catalogue of what every chat surface renders, from the running server
// ABOUTME: Generated from GET /api/surfaces/capabilities - DO NOT EDIT MANUALLY
//
// Surfaces: 7 · Reply-block kinds: 9 · Notification screens: 8
// capability-digest: 6100756fdd790e2f
// content-digest: ab895018fe88fa3b
// To regenerate: bun run generate (from packages/shared-constants)

/**
 * Every reply-block kind the server can put in a turn envelope, in the order a
 * reply lays them out.
 *
 * A surface renders the kinds its row lists and is never handed another: the
 * pipeline reads the same capabilities this file was generated from before it
 * pushes a block.
 */
export const REPLY_BLOCK_KINDS = [
  'prose',
  'activity_list',
  'workout_plan',
  'scene',
  'scene_image',
  'verdicts',
  'reconnect',
  'actions',
  'notice',
] as const;

/** One renderable piece of an assistant reply, named as the wire names it. */
export type ReplyBlockKind = (typeof REPLY_BLOCK_KINDS)[number];

/** Every surface the platform serves a chat turn on. */
export const SURFACE_CAPABILITY_IDS = [
  'web_chat',
  'mobile_chat',
  'telegram',
  'whatsapp',
  'discord',
  'slack',
  'messenger',
] as const;

/** A surface's telemetry id — the `channel` dimension on its pipeline spans. */
export type SurfaceCapabilityId = (typeof SURFACE_CAPABILITY_IDS)[number];

/** What one surface can put in front of an athlete. */
export interface SurfaceCapabilities {
  /** The surface's telemetry id. */
  id: SurfaceCapabilityId;
  /** The `call_type` stamped on this surface's LLM usage rows. */
  call_type: string;
  /** How the surface reads the natural-language part of a reply. */
  prose: 'markdown' | 'plain_text';
  /** Per-message character ceiling, or null where the transport imposes none. */
  max_reply_chars: number | null;
  /** Whether a rendered control's press reaches the platform. */
  interactive: boolean;
  /** Whether the transport can carry a reply before the turn finishes. */
  progressive: 'complete' | 'delta_channel';
  /** Whether a turn here streams partial text when the provider emits deltas. */
  streams_text_deltas: boolean;
  /** Fixed tool-loop budget, or null when coach/admin configuration resolves it. */
  max_tool_iterations: number | null;
  /** How the active model is resolved for a turn. */
  model_policy: 'use_stored' | 'override_with_env';
  /** Reply-block kinds this surface can be handed, in reply order. */
  blocks: readonly ReplyBlockKind[];
}

/** The `SurfaceProfile::resolve` table, one row per surface. */
export const SURFACE_CAPABILITIES: Record<SurfaceCapabilityId, SurfaceCapabilities> = {
  'web_chat': {
    id: 'web_chat',
    call_type: 'chat',
    prose: 'markdown',
    max_reply_chars: null,
    interactive: true,
    progressive: 'delta_channel',
    streams_text_deltas: true,
    max_tool_iterations: null,
    model_policy: 'use_stored',
    blocks: ['prose', 'activity_list', 'workout_plan', 'scene', 'verdicts', 'reconnect', 'actions', 'notice'],
  },
  'mobile_chat': {
    id: 'mobile_chat',
    call_type: 'chat',
    prose: 'markdown',
    max_reply_chars: null,
    interactive: true,
    progressive: 'delta_channel',
    streams_text_deltas: true,
    max_tool_iterations: null,
    model_policy: 'use_stored',
    blocks: ['prose', 'activity_list', 'workout_plan', 'scene', 'verdicts', 'reconnect', 'actions', 'notice'],
  },
  'telegram': {
    id: 'telegram',
    call_type: 'messaging',
    prose: 'plain_text',
    max_reply_chars: 4096,
    interactive: true,
    progressive: 'complete',
    streams_text_deltas: false,
    max_tool_iterations: 5,
    model_policy: 'override_with_env',
    blocks: ['prose', 'scene_image', 'reconnect', 'actions', 'notice'],
  },
  'whatsapp': {
    id: 'whatsapp',
    call_type: 'messaging',
    prose: 'plain_text',
    max_reply_chars: 4096,
    interactive: false,
    progressive: 'complete',
    streams_text_deltas: false,
    max_tool_iterations: 5,
    model_policy: 'override_with_env',
    blocks: ['prose', 'scene_image', 'reconnect', 'notice'],
  },
  'discord': {
    id: 'discord',
    call_type: 'messaging',
    prose: 'plain_text',
    max_reply_chars: 2000,
    interactive: true,
    progressive: 'complete',
    streams_text_deltas: false,
    max_tool_iterations: 5,
    model_policy: 'override_with_env',
    blocks: ['prose', 'scene_image', 'reconnect', 'actions', 'notice'],
  },
  'slack': {
    id: 'slack',
    call_type: 'messaging',
    prose: 'plain_text',
    max_reply_chars: 40000,
    interactive: true,
    progressive: 'complete',
    streams_text_deltas: false,
    max_tool_iterations: 5,
    model_policy: 'override_with_env',
    blocks: ['prose', 'scene_image', 'reconnect', 'actions', 'notice'],
  },
  'messenger': {
    id: 'messenger',
    call_type: 'messaging',
    prose: 'plain_text',
    max_reply_chars: 2000,
    interactive: true,
    progressive: 'complete',
    streams_text_deltas: false,
    max_tool_iterations: 5,
    model_policy: 'override_with_env',
    blocks: ['prose', 'scene_image', 'reconnect', 'actions', 'notice'],
  },
};

/**
 * The notification `data.screen` vocabulary, paired with the `USER_SURFACES`
 * id each token opens.
 *
 * The server declares this once; a client turns the surface id into its own
 * route through the registry rather than keeping a map of its own.
 */
export const NOTIFICATION_SCREEN_SURFACES = {
  'activity': 'insights',
  'activities': 'insights',
  'recovery': 'insights',
  'stats': 'insights',
  'social': 'insights',
  'coach': 'chat',
  'settings': 'profile',
  'connections': 'data-providers',
} as const;

/** A screen name a notification's `data.screen` field can carry. */
export type NotificationScreen = keyof typeof NOTIFICATION_SCREEN_SURFACES;
