// ABOUTME: Maps a resolved photograveur Scene to SVG — a switch over five node kinds
// ABOUTME: No geometry lives here; colour tokens resolve through Tailwind so both themes work

import { memo } from 'react';
import type {
  ColorToken,
  RenderBlock,
  Scene,
  SceneNode,
  TableView,
  TextRole,
} from '@pierre/scene-types';
import RouteView from './RouteView';

/**
 * Tailwind classes per colour token.
 *
 * The pillar accents are defined as CSS variables in `index.css` with a dark
 * counterpart under `prefers-color-scheme`, so naming the class is all this
 * component does about theming — no `getComputedStyle`, no palette cached at
 * module scope, and nothing to re-resolve when the theme flips.
 */
const FILL: Record<ColorToken, string> = {
  activity: 'fill-activity',
  nutrition: 'fill-nutrition',
  recovery: 'fill-recovery',
  mobility: 'fill-mobility',
  axis: 'fill-outline',
  grid: 'fill-outline-variant',
  label: 'fill-on-surface-variant',
};

const STROKE: Record<ColorToken, string> = {
  activity: 'stroke-activity',
  nutrition: 'stroke-nutrition',
  recovery: 'stroke-recovery',
  mobility: 'stroke-mobility',
  axis: 'stroke-outline',
  grid: 'stroke-outline-variant',
  label: 'stroke-on-surface-variant',
};

/** Swatch background per token, for the legend. */
const SWATCH: Record<ColorToken, string> = {
  activity: 'bg-activity',
  nutrition: 'bg-nutrition',
  recovery: 'bg-recovery',
  mobility: 'bg-mobility',
  axis: 'bg-outline',
  grid: 'bg-outline-variant',
  label: 'bg-on-surface-variant',
};

/**
 * Font size per text role, in scene units.
 *
 * The Scene deliberately carries a role rather than a size so each platform
 * picks its own; these are tuned for the 640×360 viewBox scaled into a chat
 * bubble.
 */
const FONT_SIZE: Record<TextRole, number> = {
  axis_tick: 11,
  axis_title: 12,
};

/** One primitive. This function is the entire renderer. */
function Node({ node }: { node: SceneNode }) {
  switch (node.node) {
    case 'path':
      return (
        <path
          d={node.d}
          fill="none"
          className={STROKE[node.stroke]}
          strokeWidth={node.width}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
      );
    case 'region':
      return (
        <path d={node.d} stroke="none" className={FILL[node.fill]} fillOpacity={node.opacity} />
      );
    case 'rect':
      return (
        <rect
          x={node.x}
          y={node.y}
          width={node.width}
          height={node.height}
          rx={2}
          className={FILL[node.fill]}
        />
      );
    case 'line':
      return (
        <line
          x1={node.x1}
          y1={node.y1}
          x2={node.x2}
          y2={node.y2}
          className={STROKE[node.stroke]}
          strokeWidth={node.width}
        />
      );
    case 'text':
      return (
        <text
          x={node.x}
          y={node.y}
          textAnchor={node.anchor}
          dominantBaseline={node.baseline}
          fontSize={FONT_SIZE[node.role]}
          className={FILL[node.color]}
        >
          {node.content}
        </text>
      );
    default:
      // A node kind this client does not know about — the server is ahead of
      // the deployed bundle. Skipping it degrades the chart rather than
      // blanking the whole reply.
      return null;
  }
}

/** A resolved chart. */
function ChartView({ scene }: { scene: Scene }) {
  const summary = scene.title ?? scene.legend.map((e) => e.label).join(', ');
  return (
    <figure className="my-4">
      {scene.title && (
        <figcaption className="mb-2 text-sm font-medium text-on-surface">
          {scene.title}
        </figcaption>
      )}
      <div className="overflow-x-auto">
        <svg
          viewBox={`0 0 ${scene.view_box.width} ${scene.view_box.height}`}
          preserveAspectRatio="xMidYMid meet"
          className="h-auto w-full"
          role="img"
          aria-label={`Chart: ${summary}`}
        >
          {scene.nodes.map((node, i) => (
            // Scene nodes are positional and have no identity of their own; the
            // list is regenerated wholesale on every render, so the index is a
            // stable key here rather than the usual anti-pattern.
            <Node key={i} node={node} />
          ))}
        </svg>
      </div>
      {scene.legend.length > 0 && (
        <ul className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-on-surface-variant">
          {scene.legend.map((entry) => (
            <li key={entry.label} className="flex items-center gap-1.5">
              <span className={`h-0.5 w-3 rounded-sm ${SWATCH[entry.color]}`} aria-hidden="true" />
              {entry.label}
            </li>
          ))}
        </ul>
      )}
      <p className="mt-1 text-xs text-on-surface-variant">
        source: {scene.source_tool}
      </p>
    </figure>
  );
}

/** A resolved table. Cells arrive already formatted. */
function SceneTable({ view }: { view: TableView }) {
  return (
    <figure className="my-4">
      {view.title && (
        <figcaption className="mb-2 text-sm font-medium text-on-surface">
          {view.title}
        </figcaption>
      )}
      <div className="overflow-x-auto rounded border border-outline-variant">
        <table className="w-full text-sm">
          <thead>
            <tr>
              {view.columns.map((column, i) => (
                <th
                  key={column}
                  className={`border-b border-outline-variant px-3 py-2 text-xs font-medium text-on-surface-variant ${
                    view.alignments[i] === 'right' ? 'text-right' : 'text-left'
                  }`}
                >
                  {column}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {view.rows.map((row, r) => (
              <tr key={r}>
                {row.map((cell, c) => (
                  <td
                    key={c}
                    className={`border-b border-outline-variant px-3 py-2 text-on-surface last:border-0 ${
                      view.alignments[c] === 'right'
                        ? 'text-right tabular-nums'
                        : 'text-left'
                    }`}
                  >
                    {cell}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="mt-1 text-xs text-on-surface-variant">
        source: {view.source_tool}
      </p>
    </figure>
  );
}

/**
 * Render one resolved visual block.
 *
 * Everything geometric already happened on the server: this walks a flat node
 * list and emits one element per node. There is no chart library on this side
 * and no maths — which is the whole point of resolving a Scene rather than
 * shipping the coach's spec to the client.
 *
 * A route is the one block that keeps its geometry in geographic coordinates
 * rather than arriving pre-projected, because the projection belongs to the
 * basemap it is drawn over and the athlete chooses that by panning. It is
 * therefore a map component rather than a node list — see `RouteView`.
 */
export const SceneView = memo(function SceneView({ block }: { block: RenderBlock }) {
  if (block.kind === 'chart') {
    return <ChartView scene={block} />;
  }
  if (block.kind === 'table') {
    return <SceneTable view={block} />;
  }
  if (block.kind === 'route') {
    return <RouteView view={block} />;
  }
  return null;
});
