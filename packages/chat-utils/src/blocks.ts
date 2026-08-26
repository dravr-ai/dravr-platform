// ABOUTME: Decodes a persisted transcript row into the same ReplyBlock list a live turn arrives in
// ABOUTME: So each client walks one block list, whether the turn just landed or was read back from history

import type { ClaimVerdict, Message, ReplyBlock } from '@pierre/shared-types';
import { splitActivityContent } from './activity';
import { stripToolScaffolding } from './conversation';
import { parseSceneBlocks } from './viz';

/** `kind` marking the stored scene entry that is a workout plan, not a chart. */
const WORKOUT_PLAN_KIND = 'workout_plan';

/**
 * Rebuild the reply blocks for a message the server sent no block list for.
 *
 * A live turn arrives already decomposed — the pipeline read the surface's
 * render capabilities and decided what gets its own block. A history read has
 * only the persisted row, so the same decomposition is done here, once, from
 * the columns that row carries: the activity list baked into older content,
 * the resolved scenes, and the workout plan among them.
 *
 * The `scene` block keeps `scene_blocks` verbatim rather than filtering the
 * plan out of it, because the prose's `⟦viz:N⟧` markers index that array
 * positionally and a filtered copy would shift every chart by one.
 *
 * @param message the persisted row.
 * @param verdicts the conversation's verdict rows for this message, when the
 *   surface read them. They become the reply's `verdicts` block, the same one a
 *   live turn carries.
 */
export function transcriptBlocks(
  message: Message,
  verdicts: readonly ClaimVerdict[] = [],
): ReplyBlock[] {
  const blocks: ReplyBlock[] = [];

  // Defensive: whole tool-plumbing rows are filtered upstream; this guards the
  // rarer case of scaffolding leaking into a visible turn's own content.
  const cleaned = stripToolScaffolding(message.content);

  if (message.role === 'user') {
    const asked = cleaned.trim();
    return asked.length > 0 ? [{ type: 'prose', text: asked }] : [];
  }

  const [activityList, analysis] = splitActivityContent(cleaned);

  if (activityList) {
    blocks.push({ type: 'activity_list', text: activityList });
  }

  const prose = (activityList ? analysis : cleaned).trim();
  if (prose.length > 0) {
    blocks.push({ type: 'prose', text: prose });
  }

  if (message.scene_blocks) {
    blocks.push({ type: 'scene', scene_blocks: message.scene_blocks });
  }

  const plan = parseSceneBlocks(message.scene_blocks).find(
    (block) => block.kind === WORKOUT_PLAN_KIND,
  );
  if (plan && 'plan' in plan) {
    blocks.push({ type: 'workout_plan', plan: plan.plan });
  }

  if (verdicts.length > 0) {
    blocks.push({
      type: 'verdicts',
      chips: verdicts.map((verdict) => ({
        claim: verdict.claim_text,
        contradicted: verdict.status === 'contradicted',
      })),
    });
  }

  return blocks;
}
