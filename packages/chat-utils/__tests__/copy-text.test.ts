// ABOUTME: What a reply carries onto the clipboard — no ⟦viz:N⟧ token, a caption in its place
// ABOUTME: The prose either side is the coach's own, verbatim, which is what makes a paste readable

import { describe, it, expect } from 'vitest';
import type { RenderBlock } from '@pierre/scene-types';
import { copyableText } from '../src/copy-text';

/** A `t` that shows which key and params the module asked for. */
const t = (key: string, params?: Record<string, string | number>): string =>
  params?.title ? `${key}(${String(params.title)})` : key;

const chart = (title: string | null): RenderBlock => ({
  kind: 'chart',
  view_box: { x: 0, y: 0, width: 320, height: 180 },
  nodes: [],
  legend: [],
  title,
  source_tool: 'get_activities',
});

const table = (title: string | null): RenderBlock => ({
  kind: 'table',
  columns: ['Day', 'Distance'],
  rows: [['Tuesday', '12 km']],
  alignments: ['left', 'right'],
  title,
  source_tool: 'get_activities',
});

const plan = (): RenderBlock => ({
  kind: 'workout_plan',
  plan: { name: 'Threshold Tuesday' },
  source_tool: 'generate_workout',
});

const BEFORE = 'Voici ton vélo d’août — surtout du VTT à Prévost 🚴';
const AFTER = 'Neuf sorties, environ 128 km au total.';

describe('copyableText', () => {
  it('replaces the marker with the chart’s own caption and keeps the prose verbatim', () => {
    const copied = copyableText(
      `${BEFORE}\n\n⟦viz:0⟧\n\n${AFTER}`,
      [chart('Volume hebdomadaire')],
      t,
    );

    // The token that means nothing outside the app is gone — both brackets.
    expect(copied).not.toContain('⟦');
    expect(copied).not.toContain('⟧');
    expect(copied).not.toContain('viz:0');

    // A caption stands where the chart did, naming what it showed.
    expect(copied).toContain('chat.copy.chartCaptionTitled(Volume hebdomadaire)');

    // The coach's words on both sides survive exactly as written.
    expect(copied).toBe(
      `${BEFORE}\n\nchat.copy.chartCaptionTitled(Volume hebdomadaire)\n\n${AFTER}`,
    );
  });

  it('falls back to the untitled caption when the chart carried no title', () => {
    const copied = copyableText(`${BEFORE}\n\n⟦viz:0⟧`, [chart(null)], t);

    expect(copied).toBe(`${BEFORE}\n\nchat.copy.chartCaption`);
  });

  it('names a table as a table and a plan as a plan', () => {
    expect(copyableText('⟦viz:0⟧', [table('Semaine')], t)).toBe(
      'chat.copy.tableCaptionTitled(Semaine)',
    );
    expect(copyableText('⟦viz:0⟧', [table(null)], t)).toBe('chat.copy.tableCaption');
    expect(copyableText('⟦viz:0⟧', [plan()], t)).toBe('chat.copy.planCaption');
  });

  it('keeps several captions in the order their markers appeared', () => {
    const copied = copyableText(
      'Charge :\n\n⟦viz:0⟧\n\nEt le détail :\n\n⟦viz:1⟧',
      [chart('Charge'), table('Détail')],
      t,
    );

    expect(copied).toBe(
      'Charge :\n\nchat.copy.chartCaptionTitled(Charge)\n\nEt le détail :\n\nchat.copy.tableCaptionTitled(Détail)',
    );
  });

  it('keeps two adjacent captions on their own lines', () => {
    // splitVizMarkers drops the whitespace-only run between the markers, so
    // without a separator the two captions would run together on one line.
    const copied = copyableText('⟦viz:0⟧\n\n⟦viz:1⟧', [chart('A'), chart('B')], t);

    expect(copied).toBe(
      'chat.copy.chartCaptionTitled(A)\n\nchat.copy.chartCaptionTitled(B)',
    );
  });

  it('leaves nothing behind for a marker whose block never resolved', () => {
    // Nothing was drawn there either, so a caption would name a chart the
    // athlete never saw.
    const copied = copyableText(`${BEFORE}\n\n⟦viz:3⟧\n\n${AFTER}`, [chart('Charge')], t);

    expect(copied).toBe(`${BEFORE}\n\n${AFTER}`);
    expect(copied).not.toContain('chat.copy.');
  });

  it('returns a marker-free reply untouched', () => {
    expect(copyableText(`${BEFORE}\n\n${AFTER}`, [], t)).toBe(`${BEFORE}\n\n${AFTER}`);
  });
});
