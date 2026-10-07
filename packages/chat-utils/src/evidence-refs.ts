// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A verdict's comma-separated evidence references, each with the public page it names and the study's name
// ABOUTME: Only DOI and PMID ids resolve, onto doi.org and PubMed; anything else stays unlinked

import type { EvidenceCitation } from '@pierre/shared-types';

/** One evidence reference, as stored and as an athlete can open and read it. */
export interface EvidenceRef {
  /** The id as the verifier stored it, e.g. `doi:10.1111/sms.12104`. */
  id: string;
  /** The study's public page, or null when the id is of no known kind. */
  href: string | null;
  /** Author and year, "Rønnestad & Mujika, 2014", or null when the corpus does not name it. */
  label: string | null;
  /**
   * The study's full title, followed by the journal and year it appeared in
   * when the corpus carries them ("Title — Exp Physiol, 2021"), or null when
   * the corpus carries no title.
   */
  title: string | null;
}

/**
 * What the server resolved for one reference: a verdict's `evidence` entry,
 * of which only the naming fields are read here.
 */
type StudyName = Pick<EvidenceCitation, 'id' | 'label' | 'title'> &
  Partial<Pick<EvidenceCitation, 'journal' | 'year'>>;

/** The title an athlete reads on hover, with where and when it appeared. */
function citedTitle(study: StudyName | undefined): string | null {
  const title = present(study?.title);
  if (!title) return null;
  const venue = [present(study?.journal), study?.year ?? null].filter((part) => part !== null).join(', ');
  return venue ? `${title} — ${venue}` : title;
}

/** `doi:10.1111/sms.12104`, or a bare `10.1111/sms.12104`. */
const DOI = /^(?:doi:\s*)?(10\.\d{4,9}\/\S+)$/i;
/** `pmid:22389869`. */
const PMID = /^pmid:\s*(\d{1,9})$/i;

/**
 * Where one reference id can be read.
 *
 * The address is built here from the id rather than taken from anything
 * stored — the corpus's own `url` included — so a reference can only ever
 * open doi.org or PubMed.
 */
function hrefFor(id: string): string | null {
  const doi = DOI.exec(id);
  if (doi) return `https://doi.org/${doi[1].split('/').map(encodeURIComponent).join('/')}`;
  const pmid = PMID.exec(id);
  if (pmid) return `https://pubmed.ncbi.nlm.nih.gov/${pmid[1]}/`;
  return null;
}

/** A blank or whitespace-only name is no name. */
function present(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

/**
 * Split a verdict's `evidence_refs` into its references, in stored order,
 * each with the page it opens. Blank entries and repeats are dropped.
 *
 * `studies` is the verdict's `evidence`, which the chat read resolves from
 * the evidence corpus (carnet#801): a reference it names carries that label
 * and title. A reference it does not name, or a read that carries none (the
 * admin read, a server older than the field), keeps null for both.
 */
export function parseEvidenceRefs(
  raw: string | null | undefined,
  studies?: readonly StudyName[] | null,
): EvidenceRef[] {
  const seen = new Set<string>();
  const refs: EvidenceRef[] = [];
  for (const part of (raw ?? '').split(',')) {
    const id = part.trim();
    if (!id || seen.has(id)) continue;
    seen.add(id);
    const study = studies?.find((candidate) => candidate.id === id);
    refs.push({ id, href: hrefFor(id), label: present(study?.label), title: citedTitle(study) });
  }
  return refs;
}
