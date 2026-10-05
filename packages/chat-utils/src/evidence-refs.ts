// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A verdict's comma-separated evidence references, each with the public page it names
// ABOUTME: Only DOI and PMID ids resolve, onto doi.org and PubMed; anything else stays unlinked

/** One evidence reference, as stored and as an athlete can open it. */
export interface EvidenceRef {
  /** The id as the verifier stored it, e.g. `doi:10.1111/sms.12104`. */
  id: string;
  /** The study's public page, or null when the id is of no known kind. */
  href: string | null;
}

/** `doi:10.1111/sms.12104`, or a bare `10.1111/sms.12104`. */
const DOI = /^(?:doi:\s*)?(10\.\d{4,9}\/\S+)$/i;
/** `pmid:22389869`. */
const PMID = /^pmid:\s*(\d{1,9})$/i;

/**
 * Where one reference id can be read.
 *
 * The address is built here from the id rather than taken from anything
 * stored, so a reference can only ever open doi.org or PubMed.
 */
function hrefFor(id: string): string | null {
  const doi = DOI.exec(id);
  if (doi) return `https://doi.org/${doi[1].split('/').map(encodeURIComponent).join('/')}`;
  const pmid = PMID.exec(id);
  if (pmid) return `https://pubmed.ncbi.nlm.nih.gov/${pmid[1]}/`;
  return null;
}

/**
 * Split a verdict's `evidence_refs` into its references, in stored order,
 * each with the page it opens. Blank entries and repeats are dropped.
 */
export function parseEvidenceRefs(raw: string | null | undefined): EvidenceRef[] {
  const seen = new Set<string>();
  const refs: EvidenceRef[] = [];
  for (const part of (raw ?? '').split(',')) {
    const id = part.trim();
    if (!id || seen.has(id)) continue;
    seen.add(id);
    refs.push({ id, href: hrefFor(id) });
  }
  return refs;
}
