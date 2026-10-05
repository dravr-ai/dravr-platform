// ABOUTME: Which page each evidence reference opens — doi.org for a DOI, PubMed for a PMID
// ABOUTME: An id of no known kind opens nothing, so a stored string can never pick the address

import { describe, it, expect } from 'vitest';
import { parseEvidenceRefs } from '../src/evidence-refs';

describe('parseEvidenceRefs', () => {
  it('resolves the two kinds the evidence corpus uses', () => {
    expect(parseEvidenceRefs('doi:10.1111/sms.12104, pmid:22389869')).toEqual([
      { id: 'doi:10.1111/sms.12104', href: 'https://doi.org/10.1111/sms.12104' },
      { id: 'pmid:22389869', href: 'https://pubmed.ncbi.nlm.nih.gov/22389869/' },
    ]);
  });

  it('accepts a bare DOI and escapes what a path must not carry', () => {
    expect(parseEvidenceRefs('10.1002/(SICI)1097-4636#x')).toEqual([
      { id: '10.1002/(SICI)1097-4636#x', href: 'https://doi.org/10.1002/(SICI)1097-4636%23x' },
    ]);
  });

  it('leaves an id of no known kind unlinked, a URL included', () => {
    expect(parseEvidenceRefs('seiler-2025,https://example.com/x,javascript:alert(1)')).toEqual([
      { id: 'seiler-2025', href: null },
      { id: 'https://example.com/x', href: null },
      { id: 'javascript:alert(1)', href: null },
    ]);
  });

  it('drops blanks and repeats, and reads null as no references', () => {
    expect(parseEvidenceRefs('doi:10.1234/a ,, doi:10.1234/a')).toEqual([
      { id: 'doi:10.1234/a', href: 'https://doi.org/10.1234/a' },
    ]);
    expect(parseEvidenceRefs(null)).toEqual([]);
  });
});
