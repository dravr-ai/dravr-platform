// ABOUTME: The text a coach reply becomes on the clipboard — its charts named, its ⟦viz:N⟧ markers gone
// ABOUTME: One implementation, so a copy from the phone and a copy from the browser read identically

import type { RenderBlock } from '@pierre/scene-types';
import { splitVizMarkers } from './viz';
import type { Translate } from './text';

/** A chart that carried its own caption. */
export const CHART_CAPTION_TITLED_KEY = 'chat.copy.chartCaptionTitled';
/** A chart with no caption of its own. */
export const CHART_CAPTION_KEY = 'chat.copy.chartCaption';
/** A table that carried its own caption. */
export const TABLE_CAPTION_TITLED_KEY = 'chat.copy.tableCaptionTitled';
/** A table with no caption of its own. */
export const TABLE_CAPTION_KEY = 'chat.copy.tableCaption';
/** A workout plan card, which carries no caption field. */
export const PLAN_CAPTION_KEY = 'chat.copy.planCaption';

/**
 * The line a visual block leaves behind in copied text.
 *
 * A key and its params rather than a sentence: this module is imported by both
 * clients and has no locale, so the caller translates with the `t` that knows
 * the athlete's language.
 */
interface CaptionText {
  key: string;
  params?: Record<string, string>;
}

/**
 * Name one resolved block in a single line.
 *
 * The kinds are named apart because calling a table a chart is simply wrong to
 * whoever reads the paste, and the reader has no picture to correct them with.
 */
function captionFor(block: RenderBlock): CaptionText {
  switch (block.kind) {
    case 'chart':
      return block.title
        ? { key: CHART_CAPTION_TITLED_KEY, params: { title: block.title } }
        : { key: CHART_CAPTION_KEY };
    case 'table':
      return block.title
        ? { key: TABLE_CAPTION_TITLED_KEY, params: { title: block.title } }
        : { key: TABLE_CAPTION_KEY };
    case 'workout_plan':
      return { key: PLAN_CAPTION_KEY };
  }
}

/**
 * Turn a reply into the text a clipboard or a share sheet should carry.
 *
 * On screen a ⟦viz:N⟧ marker becomes a chart; pasted into a message to a
 * training partner it is a token that means nothing anywhere. Each marker is
 * replaced by a one-line caption naming what stood there, which reads as an
 * omission the reader understands rather than as a hole or a glyph.
 *
 * The prose either side is carried verbatim — only runs of blank lines left by
 * a substitution are collapsed, and the ends are trimmed.
 *
 * A marker whose block did not resolve leaves nothing behind: nothing was
 * drawn there either, so naming it would describe something the athlete never
 * saw.
 */
export function copyableText(
  content: string,
  scenes: RenderBlock[],
  t: Translate,
): string {
  const parts: string[] = [];
  let previousWasCaption = false;

  for (const segment of splitVizMarkers(content)) {
    if (segment.kind === 'prose') {
      parts.push(segment.text);
      previousWasCaption = false;
      continue;
    }
    const block = scenes[segment.index];
    if (!block) continue;
    // Two markers with only dropped whitespace between them would otherwise
    // run their captions together on one line.
    if (previousWasCaption) parts.push('\n\n');
    const caption = captionFor(block);
    parts.push(t(caption.key, caption.params));
    previousWasCaption = true;
  }

  return parts.join('').replace(/\n{3,}/g, '\n\n').trim();
}
