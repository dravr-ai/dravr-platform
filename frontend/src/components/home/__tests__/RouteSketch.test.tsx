// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the route sketch — a real decoded polyline becomes one labelled SVG line inside its box
// ABOUTME: Red when a track that is not a shape still draws a frame, or the drawing loses its accessible name

import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { decodePolyline } from '@pierre/domain-utils';
import { RouteSketch } from '../RouteSketch';
import { SAMPLE_POLYLINE } from './homeFixtures';

describe('RouteSketch', () => {
  it('draws a decoded polyline as one path of three vertices inside the 64 × 48 box', () => {
    const points = decodePolyline(SAMPLE_POLYLINE);
    expect(points).toEqual([
      [38.5, -120.2],
      [40.7, -120.95],
      [43.252, -126.453],
    ]);
    render(<RouteSketch points={points ?? []} label="Sketch of the route" />);

    const svg = screen.getByRole('img', { name: 'Sketch of the route' });
    expect(svg).toHaveAttribute('viewBox', '0 0 64 48');
    const d = svg.querySelector('path')?.getAttribute('d') ?? '';
    const vertices = d.split(/\s*[ML]\s*/).filter(Boolean).map((pair) => pair.split(' ').map(Number));
    expect(d.startsWith('M')).toBe(true);
    expect(vertices).toHaveLength(3);
    for (const [x, y] of vertices) {
      expect(x).toBeGreaterThanOrEqual(5);
      expect(x).toBeLessThanOrEqual(59);
      expect(y).toBeGreaterThanOrEqual(5);
      expect(y).toBeLessThanOrEqual(43);
    }
    // North is up: the last point is the most northerly, so it is drawn highest.
    expect(vertices[2][1]).toBeLessThan(vertices[0][1]);
  });

  it('draws nothing for a track that is not a shape', () => {
    const { container } = render(
      <RouteSketch
        points={[
          [45.5, -73.6],
          [45.5, -73.6],
        ]}
        label="Sketch of the route"
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('draws nothing for a single fix', () => {
    const { container } = render(<RouteSketch points={[[45.5, -73.6]]} label="Sketch of the route" />);
    expect(container).toBeEmptyDOMElement();
  });
});
