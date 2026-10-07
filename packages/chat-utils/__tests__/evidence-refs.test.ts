// ABOUTME: Which page each evidence reference opens — doi.org for a DOI, PubMed for a PMID — and the study it names
// ABOUTME: An id of no known kind opens nothing, so a stored string can never pick the address

import { describe, it, expect } from 'vitest';
import { parseEvidenceRefs } from '../src/evidence-refs';

const unnamed = { label: null, title: null };

describe('parseEvidenceRefs', () => {
  it('resolves the two kinds the evidence corpus uses', () => {
    expect(parseEvidenceRefs('doi:10.1111/sms.12104, pmid:22389869')).toEqual([
      { id: 'doi:10.1111/sms.12104', href: 'https://doi.org/10.1111/sms.12104', ...unnamed },
      { id: 'pmid:22389869', href: 'https://pubmed.ncbi.nlm.nih.gov/22389869/', ...unnamed },
    ]);
  });

  it('accepts a bare DOI and escapes what a path must not carry', () => {
    expect(parseEvidenceRefs('10.1002/(SICI)1097-4636#x')).toEqual([
      { id: '10.1002/(SICI)1097-4636#x', href: 'https://doi.org/10.1002/(SICI)1097-4636%23x', ...unnamed },
    ]);
  });

  it('leaves an id of no known kind unlinked, a URL included', () => {
    expect(parseEvidenceRefs('seiler-2025,https://example.com/x,javascript:alert(1)')).toEqual([
      { id: 'seiler-2025', href: null, ...unnamed },
      { id: 'https://example.com/x', href: null, ...unnamed },
      { id: 'javascript:alert(1)', href: null, ...unnamed },
    ]);
  });

  it('drops blanks and repeats, and reads null as no references', () => {
    expect(parseEvidenceRefs('doi:10.1234/a ,, doi:10.1234/a')).toEqual([
      { id: 'doi:10.1234/a', href: 'https://doi.org/10.1234/a', ...unnamed },
    ]);
    expect(parseEvidenceRefs(null)).toEqual([]);
  });

  // carnet#801: a link read "Read the study" because nothing named it.
  it('names each study the verdict resolved, by id, in stored order', () => {
    const studies = [
      { id: 'pmid:22389869', label: 'Nielsen et al., 2012', title: 'Training errors and running related injuries' },
      { id: 'doi:10.1111/sms.12104', label: 'Rønnestad & Mujika, 2014', title: 'Optimizing strength training' },
    ];
    expect(parseEvidenceRefs('doi:10.1111/sms.12104,pmid:22389869', studies)).toEqual([
      {
        id: 'doi:10.1111/sms.12104',
        href: 'https://doi.org/10.1111/sms.12104',
        label: 'Rønnestad & Mujika, 2014',
        title: 'Optimizing strength training',
      },
      {
        id: 'pmid:22389869',
        href: 'https://pubmed.ncbi.nlm.nih.gov/22389869/',
        label: 'Nielsen et al., 2012',
        title: 'Training errors and running related injuries',
      },
    ]);
  });

  it('leaves a study unnamed when the corpus did not name it, or named it blank', () => {
    const studies = [
      { id: 'doi:10.1234/a', label: null, title: null },
      { id: 'doi:10.1234/b', label: '  ', title: '' },
    ];
    expect(parseEvidenceRefs('doi:10.1234/a,doi:10.1234/b,doi:10.1234/c', studies)).toEqual([
      { id: 'doi:10.1234/a', href: 'https://doi.org/10.1234/a', ...unnamed },
      { id: 'doi:10.1234/b', href: 'https://doi.org/10.1234/b', ...unnamed },
      { id: 'doi:10.1234/c', href: 'https://doi.org/10.1234/c', ...unnamed },
    ]);
  });

  it('never takes the address from what the server resolved', () => {
    const studies = [{ id: 'seiler-2025', label: 'Seiler, 2025', title: 'A survey', url: 'https://evil.example/' }];
    expect(parseEvidenceRefs('seiler-2025', studies)).toEqual([
      { id: 'seiler-2025', href: null, label: 'Seiler, 2025', title: 'A survey' },
    ]);
  });

  it('cites the journal and year after the title when the corpus carries them', () => {
    const studies = [
      { id: 'doi:10.1113/EP088544', label: 'Rønnestad et al., 2021', title: 'Five weeks of heat training', journal: 'Exp Physiol', year: 2021 },
      { id: 'pmid:1', label: 'A, 2000', title: 'Only a year', journal: null, year: 2000 },
      { id: 'pmid:2', label: 'B, 2001', title: null, journal: 'J Appl Physiol', year: 2001 },
    ];
    expect(parseEvidenceRefs('doi:10.1113/EP088544,pmid:1,pmid:2', studies).map((ref) => ref.title)).toEqual([
      'Five weeks of heat training — Exp Physiol, 2021',
      'Only a year — 2000',
      null,
    ]);
  });
});
