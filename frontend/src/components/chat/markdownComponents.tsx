// ABOUTME: Shared react-markdown component overrides for every chat surface
// ABOUTME: Opens links safely and lets a wide coach table scroll instead of squashing

import type { Components } from 'react-markdown';

/**
 * Component overrides passed to every `<Markdown>` in the chat surface.
 *
 * Two overrides, both about content the coach can now legitimately produce:
 *
 * - `a` opens in a new tab with `noopener noreferrer`, and breaks long URLs so
 *   a bare link cannot widen the message bubble.
 * - `table` wraps the table in its own horizontally scrollable box. The
 *   `prose` typography plugin sets `width: 100%` on tables, which makes a
 *   five-column training week squash its columns to unreadable slivers on a
 *   narrow window rather than overflowing. `!w-max` lets the table take the
 *   width its content needs and the wrapper scrolls to reach it, while
 *   `min-w-full` keeps a small table filling the bubble as before.
 *
 * The `!` modifier is required: the typography plugin's table rule and a plain
 * `w-max` utility have equal specificity, so which one wins would depend on
 * stylesheet order.
 */
export const MARKDOWN_COMPONENTS: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noopener noreferrer" className="break-all">
      {children}
    </a>
  ),
  table: ({ children }) => (
    <div className="overflow-x-auto" data-testid="markdown-table-scroll">
      <table className="!w-max min-w-full">{children}</table>
    </div>
  ),
};
