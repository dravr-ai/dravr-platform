// ABOUTME: Backward-compatible parser for activity lists baked into chat message content
// ABOUTME: Splits the old "activities + separator + analysis" format for rendering

// Separator used by the backend before the activity_list field was split out
const ACTIVITY_SEPARATOR = '\n\n---\n\n**Analysis:**\n\n';

/**
 * Splits assistant message content into an optional activity list and the main analysis body.
 *
 * Old messages have activities concatenated into content with a separator.
 * Returns `[activityList, analysisContent]` when an activity prefix is found,
 * or `[null, fullContent]` when the content has no embedded activity list.
 */
export function splitActivityContent(content: string): [string | null, string] {
  const sepIndex = content.indexOf(ACTIVITY_SEPARATOR);
  if (sepIndex === -1) return [null, content];

  const before = content.slice(0, sepIndex).trim();
  const after = content.slice(sepIndex + ACTIVITY_SEPARATOR.length).trim();

  // Only treat the prefix as an activity list when it looks like one
  if (before.startsWith('Your Activities:') || /^\d+\.\s+\[/.test(before)) {
    return [before, after];
  }
  return [null, content];
}

/**
 * Count numbered activities in an activity list markdown string.
 * Matches lines starting with "N." (e.g., "1. [Run] ...").
 */
export function countActivities(activityText: string): number {
  return (activityText.match(/^\d+\./gm) || []).length;
}
