// ABOUTME: Slide-in drawer showing full detail of a single claim verdict
// ABOUTME: Used by ClaimVerdictsTab to drill into claim text, evidence refs, and explanation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import { Button, Badge } from './ui';
import type { ClaimVerdictRow } from '../services/api/admin';

interface ClaimVerdictDrawerProps {
  verdict: ClaimVerdictRow;
  onClose: () => void;
}

function humanizeCategory(cat: string): string {
  return cat.replace(/_/g, ' ').replace(/\b\w/g, (c: string) => c.toUpperCase());
}

function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

export default function ClaimVerdictDrawer({ verdict, onClose }: ClaimVerdictDrawerProps) {
  // Close on Escape for keyboard users.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  const references = (verdict.evidence_refs ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-end bg-black/40"
      role="dialog"
      aria-modal="true"
      aria-label="Claim verdict details"
      onClick={onClose}
    >
      <div
        className="h-full w-full max-w-xl overflow-y-auto bg-white shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="sticky top-0 border-b border-outline-variant bg-white px-6 py-4">
          <div className="flex items-start justify-between">
            <div>
              <h3 className="text-lg font-semibold text-on-surface">
                Verdict detail
              </h3>
              <p className="mt-0.5 text-xs text-on-surface-variant">
                {verdict.id}
              </p>
            </div>
            <Button variant="secondary" onClick={onClose}>
              Close
            </Button>
          </div>
          <div className="mt-3 flex flex-wrap gap-2">
            <Badge variant="info">{verdict.status}</Badge>
            <Badge variant="secondary">{humanizeCategory(verdict.category)}</Badge>
            <Badge variant="warning">strength: {verdict.evidence_strength}</Badge>
            <Badge variant="secondary">layer: {verdict.layer_fired}</Badge>
            <Badge variant="secondary">
              confidence: {(verdict.confidence * 100).toFixed(0)}%
            </Badge>
          </div>
        </div>

        <div className="space-y-6 px-6 py-6 text-sm">
          <section>
            <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-on-surface-variant">
              Claim text
            </h4>
            <blockquote className="border-l-4 border-primary bg-surface-container p-3 text-on-surface">
              {verdict.claim_text}
            </blockquote>
          </section>

          {verdict.explanation ? (
            <section>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-on-surface-variant">
                Explanation
              </h4>
              <p className="text-on-surface-variant">{verdict.explanation}</p>
            </section>
          ) : null}

          {references.length > 0 ? (
            <section>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-on-surface-variant">
                Evidence references
              </h4>
              <ul className="space-y-1">
                {references.map((ref) => (
                  <li
                    key={ref}
                    className="font-mono text-xs text-on-surface-variant"
                  >
                    {ref}
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          <section>
            <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-on-surface-variant">
              Provenance
            </h4>
            <dl className="grid grid-cols-[120px_1fr] gap-x-3 gap-y-1 text-xs">
              <dt className="text-on-surface-variant">User</dt>
              <dd className="font-mono text-on-surface">{verdict.user_id}</dd>
              {verdict.coach_id ? (
                <>
                  <dt className="text-on-surface-variant">Coach</dt>
                  <dd className="font-mono text-on-surface">
                    {verdict.coach_id}
                  </dd>
                </>
              ) : null}
              {verdict.conversation_id ? (
                <>
                  <dt className="text-on-surface-variant">Conversation</dt>
                  <dd className="font-mono text-on-surface">
                    {verdict.conversation_id}
                  </dd>
                </>
              ) : null}
              {verdict.message_id ? (
                <>
                  <dt className="text-on-surface-variant">Message</dt>
                  <dd className="font-mono text-on-surface">
                    {verdict.message_id}
                  </dd>
                </>
              ) : null}
              <dt className="text-on-surface-variant">Emitted</dt>
              <dd className="text-on-surface">
                {formatTimestamp(verdict.created_at)}
              </dd>
            </dl>
          </section>
        </div>
      </div>
    </div>
  );
}
