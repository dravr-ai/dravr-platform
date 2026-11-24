// ABOUTME: Shared react-markdown component overrides for every chat surface
// ABOUTME: Opens links safely, lets a wide agent table scroll, and keeps headings on the Boreal type ramp

import type { Components } from 'react-markdown';

/**
 * Component overrides passed to every `<Markdown>` in the chat surface.
 *
 * Three overrides, all about content the agent can now legitimately produce:
 *
 * - `a` opens in a new tab with `noopener noreferrer`, and breaks long URLs so
 *   a bare link cannot widen the message bubble.
 * - `table` wraps the table in its own horizontally scrollable box. The
 *   `prose` typography plugin sets `width: 100%` on tables, which makes a
 *   five-column training week squash its columns to unreadable slivers on a
 *   narrow window rather than overflowing. `!w-max` lets the table take the
 *   width its content needs and the wrapper scrolls to reach it, while
 *   `min-w-full` keeps a small table filling the bubble as before.
 * - `h1`–`h4` sit on the type ramp (DESIGN.md "Hierarchy (web)"). The plugin
 *   sizes headings in em of the 15px reply, which puts a `###` at 19.3px and
 *   a `#` at 32px, louder than the column titles around the thread. A reply's largest
 *   heading is the 18px title step; then the 16px lead, a bold line at the
 *   15px reading size, and the 13px section title. All are 600, the
 *   headline weight.
 *
 * The `!` modifier is required: the typography plugin's table and heading
 * rules and a plain utility have equal specificity, so which one wins would
 * depend on stylesheet order.
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
  h1: ({ children }) => <h1 className="!text-xl !font-semibold">{children}</h1>,
  h2: ({ children }) => <h2 className="!text-lg !font-semibold">{children}</h2>,
  h3: ({ children }) => <h3 className="!text-base !font-semibold">{children}</h3>,
  h4: ({ children }) => <h4 className="!text-sm !font-semibold">{children}</h4>,
};
