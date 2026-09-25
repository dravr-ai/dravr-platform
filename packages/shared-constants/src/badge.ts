// ABOUTME: What an unread or notification badge prints for a count, on every badge both clients draw
// ABOUTME: One cap, so a tab, a bell and a conversation row never disagree about where the figure stops

/** The largest count a badge prints as a figure; anything above reads as this plus `+`. */
const BADGE_CAP = 99;

/** What a badge prints for a count; three digits is where it stops growing. */
export function badgeLabel(count: number): string {
  return count > BADGE_CAP ? `${BADGE_CAP}+` : String(count);
}
