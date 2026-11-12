// ABOUTME: The envelope notice under a reply: what the envelope check found on the turn's sessions
// ABOUTME: The platform's own translated sentences, drawn as written; the agent's prose explains, this states

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { AlertTriangle } from 'lucide-react';

interface EnvelopeNoticeProps {
  /** The notice's heading, as the server rendered it in the athlete's locale. */
  heading: string;
  /** One line per session: its day, what was found, a relabel and its anchor. */
  lines: string[];
}

/**
 * A session saved as written but outside what the evidence supports
 * (carnet#1495). Every string arrives rendered: the client lays it out and
 * adds no words of its own.
 */
export default function EnvelopeNotice({ heading, lines }: EnvelopeNoticeProps) {
  return (
    <section
      role="note"
      aria-label={heading}
      className="mt-3 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-on-surface"
    >
      <div className="flex items-center gap-2 font-medium text-on-warning-container">
        <AlertTriangle className="h-4 w-4 shrink-0" aria-hidden="true" />
        <span>{heading}</span>
      </div>
      <ul className="mt-1.5 space-y-1 pl-6">
        {lines.map((line, index) => (
          <li key={`${index}-${line.slice(0, 24)}`}>{line}</li>
        ))}
      </ul>
    </section>
  );
}
