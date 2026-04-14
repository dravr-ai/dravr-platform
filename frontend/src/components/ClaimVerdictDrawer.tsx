// ABOUTME: Slide-in drawer showing full detail of a single Tier 5.5 claim verdict
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
        className="h-full w-full max-w-xl overflow-y-auto bg-white shadow-xl dark:bg-gray-900"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="sticky top-0 border-b border-gray-200 bg-white px-6 py-4 dark:border-gray-700 dark:bg-gray-900">
          <div className="flex items-start justify-between">
            <div>
              <h3 className="text-lg font-semibold text-gray-900 dark:text-on-surface">
                Verdict detail
              </h3>
              <p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">
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
            <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Claim text
            </h4>
            <blockquote className="border-l-4 border-pierre-violet bg-gray-50 p-3 text-gray-900 dark:bg-gray-800 dark:text-gray-100">
              {verdict.claim_text}
            </blockquote>
          </section>

          {verdict.explanation ? (
            <section>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
                Explanation
              </h4>
              <p className="text-gray-700 dark:text-gray-300">{verdict.explanation}</p>
            </section>
          ) : null}

          {references.length > 0 ? (
            <section>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
                Evidence references
              </h4>
              <ul className="space-y-1">
                {references.map((ref) => (
                  <li
                    key={ref}
                    className="font-mono text-xs text-gray-700 dark:text-gray-300"
                  >
                    {ref}
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          <section>
            <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Provenance
            </h4>
            <dl className="grid grid-cols-[120px_1fr] gap-x-3 gap-y-1 text-xs">
              <dt className="text-gray-500 dark:text-gray-400">User</dt>
              <dd className="font-mono text-gray-900 dark:text-gray-100">{verdict.user_id}</dd>
              {verdict.coach_id ? (
                <>
                  <dt className="text-gray-500 dark:text-gray-400">Coach</dt>
                  <dd className="font-mono text-gray-900 dark:text-gray-100">
                    {verdict.coach_id}
                  </dd>
                </>
              ) : null}
              {verdict.conversation_id ? (
                <>
                  <dt className="text-gray-500 dark:text-gray-400">Conversation</dt>
                  <dd className="font-mono text-gray-900 dark:text-gray-100">
                    {verdict.conversation_id}
                  </dd>
                </>
              ) : null}
              {verdict.message_id ? (
                <>
                  <dt className="text-gray-500 dark:text-gray-400">Message</dt>
                  <dd className="font-mono text-gray-900 dark:text-gray-100">
                    {verdict.message_id}
                  </dd>
                </>
              ) : null}
              <dt className="text-gray-500 dark:text-gray-400">Emitted</dt>
              <dd className="text-gray-900 dark:text-gray-100">
                {formatTimestamp(verdict.created_at)}
              </dd>
            </dl>
          </section>
        </div>
      </div>
    </div>
  );
}
