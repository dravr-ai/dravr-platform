// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The reply a verdict was drawn from, cut to a short plain-text window around the claim
// ABOUTME: Markdown, tool scaffolding and ⟦viz:N⟧ markers dropped, so a preview reads as the athlete saw it

import { stripToolScaffolding } from './conversation';
import { splitVizMarkers } from './viz';

/**
 * A reply cut down to the passage around one claim.
 *
 * `match` is the claim as it stands in the reply, empty when the reply no
 * longer carries it verbatim — then `before` is the reply's opening and
 * `after` is empty, so the preview still shows what the coach said.
 */
export interface ClaimContext {
  /** The text leading up to the claim, `…`-prefixed when it was cut. */
  before: string;
  /** The claim as it reads in the reply; empty when it was not found. */
  match: string;
  /** The text after the claim, `…`-suffixed when it was cut. */
  after: string;
}

/** Characters of context kept either side of the claim. */
const DEFAULT_RADIUS = 160;

/**
 * Reduce stored reply content to the plain prose the bubble showed.
 *
 * Only the markdown a coach reply actually uses is undone — headings, list
 * bullets, quotes, emphasis, inline code and links — and every run of
 * whitespace becomes one space, because a preview is one paragraph.
 */
function plainProse(content: string): string {
  const prose = splitVizMarkers(stripToolScaffolding(content))
    .map((segment) => (segment.kind === 'prose' ? segment.text : ' '))
    .join('');
  return prose
    .replace(/^\s{0,3}#{1,6}\s+/gm, '')
    .replace(/^\s*>\s?/gm, '')
    .replace(/^\s*(?:[-*+]|\d+[.)])\s+/gm, '')
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/(\*\*|__)(.+?)\1/g, '$2')
    .replace(/(^|[^\w*])\*(?!\s)([^*\n]+?)\*(?!\w)/g, '$1$2')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Cut `text` to at most `limit` characters from its start, on a word boundary. */
function headOf(text: string, limit: number): string {
  if (text.length <= limit) return text;
  const cut = text.lastIndexOf(' ', limit);
  return `${text.slice(0, cut > 0 ? cut : limit).trimEnd()}…`;
}

/** Cut `text` to at most `limit` characters from its end, on a word boundary. */
function tailOf(text: string, limit: number): string {
  if (text.length <= limit) return text;
  const from = text.length - limit;
  const cut = text.indexOf(' ', from);
  return `…${text.slice(cut >= 0 ? cut + 1 : from).trimStart()}`;
}

/**
 * Locate `claim` in the reply `content` and keep `radius` characters either
 * side of it.
 *
 * The claim is matched after both sides are reduced to plain prose, first
 * exactly and then ignoring case, since the verifier stores the sentence
 * verbatim but the reply around it carries markdown the claim does not.
 */
export function claimInContext(content: string, claim: string, radius = DEFAULT_RADIUS): ClaimContext {
  const text = plainProse(content);
  const needle = plainProse(claim);
  let index = needle ? text.indexOf(needle) : -1;
  if (index < 0 && needle) index = text.toLowerCase().indexOf(needle.toLowerCase());
  if (index < 0) {
    return { before: headOf(text, radius * 2), match: '', after: '' };
  }
  const end = index + needle.length;
  return {
    before: tailOf(text.slice(0, index), radius),
    match: text.slice(index, end),
    after: headOf(text.slice(end), radius),
  };
}
