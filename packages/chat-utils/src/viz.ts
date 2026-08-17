// ABOUTME: Splits a reply on its ⟦viz:N⟧ markers and parses the resolved scene blocks
// ABOUTME: One implementation so web and mobile interleave prose and charts identically

import type { RenderBlock } from '@pierre/scene-types';

/**
 * A run of prose, or the position where a visual block belongs.
 *
 * Rendering a reply means walking this list in order: prose segments through the
 * usual markdown renderer, viz segments through the scene renderer.
 */
export type VizSegment =
  | { kind: 'prose'; text: string }
  | { kind: 'viz'; index: number };

/**
 * Matches the positional marker the extraction stage leaves behind.
 *
 * The brackets are U+27E6 / U+27E7 rather than ASCII precisely so ordinary coach
 * prose — including markdown, code and LaTeX-ish notation — cannot collide with
 * one. Keep this in lockstep with `marker()` in the pipeline's `viz_blocks.rs`.
 */
const MARKER = /⟦viz:(\d+)⟧/g;

/**
 * Split reply text into prose runs and block positions.
 *
 * Returns a single prose segment when the text carries no markers, so a caller
 * can use this unconditionally rather than branching first.
 *
 * Empty prose runs are dropped: two adjacent markers, or a marker at the very
 * start or end, would otherwise produce blank segments that render as stray
 * vertical gaps between charts.
 */
export function splitVizMarkers(text: string): VizSegment[] {
  const segments: VizSegment[] = [];
  let cursor = 0;

  // `matchAll` rather than a stateful `exec` loop: MARKER carries the global
  // flag, and a shared regex object with `lastIndex` is a classic source of
  // every-other-call-fails bugs.
  for (const match of text.matchAll(MARKER)) {
    const at = match.index ?? 0;
    const before = text.slice(cursor, at);
    if (before.trim().length > 0) {
      segments.push({ kind: 'prose', text: before });
    }
    segments.push({ kind: 'viz', index: Number(match[1]) });
    cursor = at + match[0].length;
  }

  const tail = text.slice(cursor);
  if (tail.trim().length > 0) {
    segments.push({ kind: 'prose', text: tail });
  }
  return segments;
}

/**
 * Parse the `scene_blocks` field into resolved blocks.
 *
 * Returns `[]` for anything unparseable rather than throwing: the field is
 * decoration on top of a reply that stands on its own, so a malformed payload
 * must cost the athlete the charts and nothing else. Mirrors `parseWorkoutPlan`
 * in `@pierre/shared-types`.
 */
export function parseSceneBlocks(sceneBlocks: string | undefined): RenderBlock[] {
  if (!sceneBlocks) {
    return [];
  }
  try {
    const parsed: unknown = JSON.parse(sceneBlocks);
    return Array.isArray(parsed) ? (parsed as RenderBlock[]) : [];
  } catch {
    return [];
  }
}

/**
 * Look up the block a `viz` segment points at.
 *
 * A marker whose block did not survive resolution returns `undefined`, and the
 * caller renders nothing there. The pipeline drops markers and blocks together
 * when a reply is rewritten, so this is the residual case of one block failing
 * to resolve while its siblings succeeded — the prose still reads correctly
 * without it.
 */
export function blockAt(
  blocks: RenderBlock[],
  index: number
): RenderBlock | undefined {
  return blocks[index];
}
