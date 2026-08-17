// ABOUTME: Marker splitting and scene-block parsing — the seam where prose and charts interleave
// ABOUTME: Ordering and the malformed-payload path are what break silently, so both are pinned

import { describe, it, expect } from 'vitest';
import { splitVizMarkers, parseSceneBlocks, blockAt } from '../src/viz';

describe('splitVizMarkers', () => {
  it('interleaves prose and block positions in reading order', () => {
    const segments = splitVizMarkers(
      'Ta charge grimpe depuis trois semaines.\n\n⟦viz:0⟧\n\nC’est pourquoi on coupe jeudi.',
    );

    expect(segments).toHaveLength(3);
    expect(segments[0]).toEqual({
      kind: 'prose',
      text: 'Ta charge grimpe depuis trois semaines.\n\n',
    });
    expect(segments[1]).toEqual({ kind: 'viz', index: 0 });
    expect(segments[2]?.kind).toBe('prose');
  });

  it('keeps several blocks in order and does not renumber them', () => {
    const segments = splitVizMarkers('A ⟦viz:0⟧ B ⟦viz:2⟧ C ⟦viz:1⟧');
    const indices = segments
      .filter((s): s is { kind: 'viz'; index: number } => s.kind === 'viz')
      .map((s) => s.index);

    // The order they appear in the prose is the order they render; the index is
    // a lookup into the block array, not a position.
    expect(indices).toEqual([0, 2, 1]);
  });

  it('returns a single prose run when there are no markers', () => {
    const segments = splitVizMarkers('Just a reply with no charts.');
    expect(segments).toEqual([{ kind: 'prose', text: 'Just a reply with no charts.' }]);
  });

  it('drops empty prose runs around adjacent markers', () => {
    // Two charts back to back would otherwise emit a blank segment between
    // them, which renders as a stray vertical gap.
    const segments = splitVizMarkers('⟦viz:0⟧⟦viz:1⟧');
    expect(segments).toEqual([
      { kind: 'viz', index: 0 },
      { kind: 'viz', index: 1 },
    ]);
  });

  it('is not confused by prose that merely mentions the marker syntax', () => {
    // ASCII brackets must not match — the markers use U+27E6/U+27E7 precisely so
    // ordinary prose, code and markdown cannot collide with them.
    const segments = splitVizMarkers('The placeholder looks like [[viz:0]] in ASCII.');
    expect(segments).toHaveLength(1);
    expect(segments[0]?.kind).toBe('prose');
  });

  it('handles a marker with no surrounding prose at all', () => {
    expect(splitVizMarkers('⟦viz:3⟧')).toEqual([{ kind: 'viz', index: 3 }]);
  });

  it('is stable across repeated calls', () => {
    // A module-scope regex carrying the global flag advances `lastIndex`
    // between `exec` calls; the every-other-call-fails bug that produces is
    // invisible in a single-call test.
    const text = 'A ⟦viz:0⟧ B';
    expect(splitVizMarkers(text)).toEqual(splitVizMarkers(text));
    expect(splitVizMarkers(text)).toEqual(splitVizMarkers(text));
  });
});

describe('parseSceneBlocks', () => {
  it('parses a resolved block array', () => {
    const encoded = JSON.stringify([
      { kind: 'table', columns: ['a', 'b'], rows: [], alignments: [], title: null, source_tool: 't' },
    ]);
    const blocks = parseSceneBlocks(encoded);
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.kind).toBe('table');
  });

  it('returns an empty array for undefined', () => {
    expect(parseSceneBlocks(undefined)).toEqual([]);
  });

  it('returns an empty array rather than throwing on malformed JSON', () => {
    // The charts are decoration on a reply that stands on its own. A broken
    // payload must cost the athlete the charts and nothing else.
    expect(parseSceneBlocks('{not json')).toEqual([]);
  });

  it('returns an empty array when the payload is not an array', () => {
    expect(parseSceneBlocks('{"kind":"chart"}')).toEqual([]);
  });
});

describe('blockAt', () => {
  it('returns undefined for a marker whose block did not resolve', () => {
    // One block failing to resolve while its siblings succeed: the prose still
    // reads correctly with nothing rendered at that position.
    expect(blockAt([], 0)).toBeUndefined();
    expect(blockAt([], 5)).toBeUndefined();
  });
});
