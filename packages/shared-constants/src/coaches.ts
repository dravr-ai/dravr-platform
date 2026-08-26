// ABOUTME: Coach tuning bounds shared by the web and mobile coach editors
// ABOUTME: Mirrors pierre_core::constants::tool_execution — the server rejects values outside them

/**
 * Smallest tool-loop iteration budget a coach may be given. One pass still
 * lets the model call a tool and answer from its result.
 */
export const MIN_MAX_TOOL_ITERATIONS = 1;

/**
 * Largest tool-loop iteration budget a coach may be given. Caps how long one
 * chat turn can spend fanning out tool calls before it must answer.
 */
export const MAX_MAX_TOOL_ITERATIONS = 50;

/**
 * Budget a coach runs on when it carries no per-coach override — the value the
 * tenant-wide `tool_execution.max_iterations` setting itself defaults to.
 */
export const DEFAULT_MAX_TOOL_ITERATIONS = 10;
