// ABOUTME: User-facing drawer showing the Tier 5.5 verdict detail for a single chat message
// ABOUTME: Includes the "Ask me about this claim" call-to-action that pre-fills the chat input
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import type { ChatVerdictRow } from '@pierre/api-client';

interface ChatVerdictDrawerProps {
  verdict: ChatVerdictRow;
  onClose: () => void;
  onAskAboutClaim?: () => void;
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

function statusToneClass(status: ChatVerdictRow['status']): string {
  switch (status) {
    case 'supported':
      return 'bg-green-500/15 text-green-300';
    case 'unsupported':
      return 'bg-amber-500/15 text-amber-300';
    case 'contradicted':
      return 'bg-red-500/15 text-red-300';
    case 'rhetorical':
      return 'bg-sky-500/15 text-sky-300';
    case 'unverifiable':
    default:
      return 'bg-zinc-500/15 text-zinc-300';
  }
}

export default function ChatVerdictDrawer({
  verdict,
  onClose,
  onAskAboutClaim,
}: ChatVerdictDrawerProps) {
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
      className="fixed inset-0 z-50 flex items-start justify-end bg-black/60"
      role="dialog"
      aria-modal="true"
      aria-label="Claim verdict details"
      onClick={onClose}
    >
      <div
        className="h-full w-full max-w-md overflow-y-auto bg-zinc-950 shadow-xl text-zinc-100"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="sticky top-0 border-b border-white/10 bg-zinc-950/90 px-5 py-4 backdrop-blur">
          <div className="flex items-start justify-between">
            <div>
              <h3 className="text-lg font-semibold text-white">About this claim</h3>
              <p className="mt-0.5 text-xs text-zinc-500">{verdict.id}</p>
            </div>
            <button
              type="button"
              onClick={onClose}
              className="rounded p-1 text-zinc-400 hover:bg-white/10 hover:text-white"
              aria-label="Close"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M6 18L18 6M6 6l12 12"
                />
              </svg>
            </button>
          </div>
          <div className="mt-3 flex flex-wrap gap-2 text-xs">
            <span className={`rounded-full px-2 py-0.5 ${statusToneClass(verdict.status)}`}>
              {verdict.status}
            </span>
            <span className="rounded-full bg-white/10 px-2 py-0.5 text-zinc-300">
              {humanizeCategory(verdict.category)}
            </span>
            <span className="rounded-full bg-white/10 px-2 py-0.5 text-zinc-300">
              evidence: {verdict.evidence_strength}
            </span>
            <span className="rounded-full bg-white/10 px-2 py-0.5 text-zinc-300">
              {verdict.layer_fired}
            </span>
          </div>
        </div>

        <div className="space-y-5 px-5 py-5 text-sm">
          <section>
            <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-zinc-500">
              The claim
            </h4>
            <blockquote className="border-l-2 border-pierre-violet bg-white/5 p-3 text-zinc-100">
              {verdict.claim_text}
            </blockquote>
          </section>

          {verdict.explanation ? (
            <section>
              <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-zinc-500">
                What the detector found
              </h4>
              <p className="text-zinc-300">{verdict.explanation}</p>
            </section>
          ) : null}

          {references.length > 0 ? (
            <section>
              <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-zinc-500">
                Evidence references
              </h4>
              <ul className="space-y-1">
                {references.map((ref) => (
                  <li key={ref} className="font-mono text-xs text-zinc-300">
                    {ref}
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          <section className="text-xs text-zinc-500">
            Verdict emitted {formatTimestamp(verdict.created_at)}
          </section>

          {onAskAboutClaim ? (
            <button
              type="button"
              onClick={onAskAboutClaim}
              className="w-full rounded-lg bg-pierre-violet px-4 py-2 text-sm font-medium text-white hover:bg-pierre-violet/90"
            >
              Ask me about this claim
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
