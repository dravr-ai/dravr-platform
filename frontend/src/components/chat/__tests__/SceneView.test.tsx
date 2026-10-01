// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the web chart and table blocks — caption, legend, cells, and no tool attribution line
// ABOUTME: Red if a chat chart or table prints the server-side tool name ("source: …") under the block again

import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { RenderBlock } from '@pierre/scene-types';

import { SceneView } from '../SceneView';

const CHART: RenderBlock = {
  kind: 'chart',
  view_box: { width: 320, height: 180 },
  nodes: [],
  legend: [{ label: 'Load', color: 'activity' }],
  title: 'Weekly load, last four weeks',
  source_tool: 'get_activities',
};

const TABLE: RenderBlock = {
  kind: 'table',
  columns: ['Week', 'Load'],
  rows: [
    ['Week 1', '412'],
    ['Week 2', '455'],
  ],
  alignments: ['left', 'right'],
  title: 'Load by week',
  source_tool: 'get_training_load',
};

describe('SceneView', () => {
  it('draws a chart with its caption and legend, and no source line', () => {
    const { container } = render(<SceneView block={CHART} />);
    expect(screen.getByRole('img', { name: 'Chart: Weekly load, last four weeks' })).toBeInTheDocument();
    expect(screen.getByText('Load')).toBeInTheDocument();
    expect(container.textContent).not.toContain('source');
    expect(container.textContent).not.toContain('get_activities');
  });

  it('draws a table with its cells, and no source line', () => {
    const { container } = render(<SceneView block={TABLE} />);
    expect(screen.getByText('Load by week')).toBeInTheDocument();
    expect(screen.getAllByRole('row')).toHaveLength(3);
    expect(screen.getByText('455')).toBeInTheDocument();
    expect(container.textContent).not.toContain('source');
    expect(container.textContent).not.toContain('get_training_load');
  });
});
