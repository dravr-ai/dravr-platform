// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile chart and table blocks — caption, legend, cells, and no tool attribution line
// ABOUTME: Red if a chat chart or table prints the server-side tool name ("source: …") under the block again

import React from 'react';
import { render, screen } from '@testing-library/react-native';
import type { RenderBlock } from '@pierre/scene-types';

import SceneView from '../SceneView';

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
    render(<SceneView block={CHART} />);
    expect(screen.getByText('Weekly load, last four weeks')).toBeTruthy();
    expect(screen.getByText('Load')).toBeTruthy();
    expect(screen.queryByText(/source/)).toBeNull();
    expect(screen.queryByText(/get_activities/)).toBeNull();
  });

  it('draws a table with its cells, and no source line', () => {
    render(<SceneView block={TABLE} />);
    expect(screen.getByText('Load by week')).toBeTruthy();
    expect(screen.getByText('455')).toBeTruthy();
    expect(screen.queryByText(/source/)).toBeNull();
    expect(screen.queryByText(/get_training_load/)).toBeNull();
  });
});
