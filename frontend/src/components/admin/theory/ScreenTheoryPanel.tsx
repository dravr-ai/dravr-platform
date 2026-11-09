// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The collapsible "How this screen works" panel at the top of every admin screen (carnet#1441)
// ABOUTME: Reads one entry of SCREEN_THEORY: the question, the theory, how the numbers are computed, since when, and the commits

import { useState } from 'react';
import { SCREEN_THEORY } from './screenTheory';

interface ScreenTheoryPanelProps {
  /** The dashboard tab id the panel describes. A tab with no entry draws nothing. */
  tab: string;
}

/**
 * Closed by default: an operator who knows the screen sees one line, and one
 * who does not opens it where the numbers are. A native `<details>` keeps it
 * keyboard-operable and announced as a disclosure. The body is mounted only
 * while open, so a closed panel adds one line of text to the screen and no
 * hidden copy for a search or a text locator to land on. Colours are theme
 * tokens only, so it follows light and dark like the screen under it. Admin
 * screens are English-only, so the copy is plain text.
 */
export function ScreenTheoryPanel({ tab }: ScreenTheoryPanelProps) {
  const [open, setOpen] = useState(false);
  const entry = SCREEN_THEORY[tab];
  if (!entry) return null;

  return (
    <details
      data-testid="screen-theory"
      data-tab={tab}
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
      className="group mb-6 rounded-lg border ghost-border bg-surface-container-low"
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-3 text-sm [&::-webkit-details-marker]:hidden">
        <svg
          aria-hidden="true"
          viewBox="0 0 20 20"
          fill="currentColor"
          className="h-4 w-4 shrink-0 text-on-surface-variant transition-transform group-open:rotate-90"
        >
          <path fillRule="evenodd" d="M7.21 14.77a.75.75 0 0 1 .02-1.06L11.17 10 7.23 6.29a.75.75 0 1 1 1.04-1.08l4.5 4.25a.75.75 0 0 1 0 1.08l-4.5 4.25a.75.75 0 0 1-1.06-.02Z" clipRule="evenodd" />
        </svg>
        <span className="font-semibold text-on-surface">How this screen works</span>
      </summary>
      {open && (
        <div className="space-y-4 px-4 pb-4 pl-10 text-sm text-on-surface-variant">
          <div>
            <p className="font-semibold text-on-surface">Question</p>
            <p className="mt-1">{entry.question}</p>
          </div>
          <div>
            <p className="font-semibold text-on-surface">Theory</p>
            <p className="mt-1">{entry.theory}</p>
          </div>
          <div>
            <p className="font-semibold text-on-surface">How the numbers are computed</p>
            <ul className="mt-1 list-disc space-y-1 pl-5">
              {entry.computation.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          </div>
          <div>
            <p className="font-semibold text-on-surface">Since</p>
            <p className="mt-1" data-testid="screen-theory-since">{entry.since}</p>
          </div>
          <div>
            <p className="font-semibold text-on-surface">Commits</p>
            <ul className="mt-1 space-y-1">
              {entry.commits.map((commit) => (
                <li key={commit.sha} className="flex flex-wrap gap-x-2">
                  <code className="font-mono text-on-surface">{commit.sha}</code>
                  <span>{commit.date}</span>
                  <span className="min-w-0 flex-1 truncate" title={commit.subject}>{commit.subject}</span>
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}
    </details>
  );
}
