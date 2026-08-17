// ABOUTME: Maps a resolved photograveur Scene to react-native-svg — the same five-kind switch as web
// ABOUTME: Colour tokens resolve through the theme palette, so one Scene is correct in both schemes

import React, { useMemo } from 'react';
import { View, Text, ScrollView } from 'react-native';
import Svg, { Path, Rect, Line, Text as SvgText } from 'react-native-svg';
import type {
  ColorToken,
  RenderBlock,
  Scene,
  SceneNode,
  TableView,
  TextRole,
} from '@pierre/scene-types';

import { useThemeColors } from '../../constants/theme';

/** Matches how the other chat components name the palette. */
type ThemeColors = ReturnType<typeof useThemeColors>;

/**
 * Resolve a scene colour token against the active palette.
 *
 * The four pillar accents come from the theme, which reads them from
 * `@pierre/shared-constants` — the same values the web renderer gets from its
 * CSS variables. Chrome tokens map onto the Boreal outline/variant tokens
 * rather than being invented here.
 */
function paletteFor(colors: ThemeColors): Record<ColorToken, string> {
  return {
    activity: colors.pierre.activity,
    nutrition: colors.pierre.nutrition,
    recovery: colors.pierre.recovery,
    mobility: colors.pierre.mobility,
    axis: colors.tokens.outline,
    grid: colors.tokens.outlineVariant,
    label: colors.tokens.onSurfaceVariant,
  };
}

/**
 * Font size per text role, in scene units.
 *
 * Slightly larger than the web's, because the same 640-unit viewBox is scaled
 * into a much narrower container on a phone and the ticks would otherwise fall
 * below legible size.
 */
const FONT_SIZE: Record<TextRole, number> = {
  axis_tick: 13,
  axis_title: 14,
};

/** One primitive. This function is the entire renderer. */
function Node({ node, palette }: { node: SceneNode; palette: Record<ColorToken, string> }) {
  switch (node.node) {
    case 'path':
      return (
        <Path
          d={node.d}
          fill="none"
          stroke={palette[node.stroke]}
          strokeWidth={node.width}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
      );
    case 'region':
      return (
        <Path d={node.d} fill={palette[node.fill]} fillOpacity={node.opacity} stroke="none" />
      );
    case 'rect':
      return (
        <Rect
          x={node.x}
          y={node.y}
          width={node.width}
          height={node.height}
          rx={2}
          fill={palette[node.fill]}
        />
      );
    case 'line':
      return (
        <Line
          x1={node.x1}
          y1={node.y1}
          x2={node.x2}
          y2={node.y2}
          stroke={palette[node.stroke]}
          strokeWidth={node.width}
        />
      );
    case 'text':
      return (
        <SvgText
          x={node.x}
          y={node.y}
          textAnchor={node.anchor}
          // react-native-svg supports alignmentBaseline rather than
          // dominant-baseline; the Scene's three values map onto it directly.
          alignmentBaseline={node.baseline === 'alphabetic' ? 'baseline' : node.baseline}
          fontSize={FONT_SIZE[node.role]}
          fill={palette[node.color]}
        >
          {node.content}
        </SvgText>
      );
    default:
      // A node kind this build does not know — the server is ahead of the
      // shipped bundle. Skipping degrades the chart rather than blanking the
      // whole reply.
      return null;
  }
}

function ChartView({ scene, colors }: { scene: Scene; colors: ThemeColors }) {
  const palette = useMemo(() => paletteFor(colors), [colors]);
  const summary = scene.title ?? scene.legend.map((e) => e.label).join(', ');

  return (
    <View className="my-3">
      {scene.title ? (
        <Text className="mb-2 text-sm font-medium" style={{ color: colors.tokens.onSurface }}>
          {scene.title}
        </Text>
      ) : null}
      <View
        className="w-full overflow-hidden rounded-lg"
        // The aspect ratio lives on this View, not on the Svg. react-native-svg
        // has no intrinsic size, so a ratio set on the Svg alone leaves the
        // parent with no measured height and the layout depending on the
        // native view sizing itself. Giving the container the ratio and letting
        // the Svg fill it makes the row's height deterministic.
        style={{ aspectRatio: scene.view_box.width / scene.view_box.height }}
        accessible
        accessibilityRole="image"
        accessibilityLabel={`Chart: ${summary}`}
      >
        <Svg
          width="100%"
          height="100%"
          viewBox={`0 0 ${scene.view_box.width} ${scene.view_box.height}`}
          preserveAspectRatio="xMidYMid meet"
        >
          {scene.nodes.map((node, i) => (
            // Scene nodes are positional and carry no identity; the whole list
            // is regenerated on every render, so the index is a stable key.
            <Node key={i} node={node} palette={palette} />
          ))}
        </Svg>
      </View>
      {scene.legend.length > 0 ? (
        <View className="mt-2 flex-row flex-wrap">
          {scene.legend.map((entry) => (
            <View key={entry.label} className="mr-4 mb-1 flex-row items-center">
              <View
                className="mr-1.5 h-0.5 w-3 rounded-sm"
                style={{ backgroundColor: palette[entry.color] }}
              />
              <Text className="text-xs" style={{ color: colors.tokens.onSurfaceVariant }}>
                {entry.label}
              </Text>
            </View>
          ))}
        </View>
      ) : null}
      <Text className="mt-1 text-[11px]" style={{ color: colors.tokens.outline }}>
        source: {scene.source_tool}
      </Text>
    </View>
  );
}

/**
 * A resolved table.
 *
 * Horizontally scrollable because a phone cannot show eight columns, and the
 * alternative — wrapping cells — destroys the row alignment that makes a table
 * readable at all.
 */
function SceneTable({ view, colors }: { view: TableView; colors: ThemeColors }) {
  return (
    <View className="my-3">
      {view.title ? (
        <Text className="mb-2 text-sm font-medium" style={{ color: colors.tokens.onSurface }}>
          {view.title}
        </Text>
      ) : null}
      <ScrollView horizontal showsHorizontalScrollIndicator={false}>
        <View
          className="rounded-lg border"
          style={{ borderColor: colors.tokens.outlineVariant }}
        >
          <View
            className="flex-row border-b"
            style={{ borderColor: colors.tokens.outlineVariant }}
          >
            {view.columns.map((column, i) => (
              <Text
                key={column}
                className="min-w-[96px] px-3 py-2 text-xs font-medium uppercase"
                style={{
                  color: colors.tokens.onSurfaceVariant,
                  textAlign: view.alignments[i] === 'right' ? 'right' : 'left',
                }}
              >
                {column}
              </Text>
            ))}
          </View>
          {view.rows.map((row, r) => (
            <View key={r} className="flex-row">
              {row.map((cell, c) => (
                <Text
                  key={c}
                  className="min-w-[96px] px-3 py-2 text-sm"
                  style={{
                    color: colors.tokens.onSurface,
                    textAlign: view.alignments[c] === 'right' ? 'right' : 'left',
                  }}
                >
                  {cell}
                </Text>
              ))}
            </View>
          ))}
        </View>
      </ScrollView>
      <Text className="mt-1 text-[11px]" style={{ color: colors.tokens.outline }}>
        source: {view.source_tool}
      </Text>
    </View>
  );
}

/**
 * Render one resolved visual block.
 *
 * Everything geometric happened on the server. This walks a flat node list and
 * emits one element per node — no chart library, no maths, and the same five
 * cases the web renderer handles.
 */
export default function SceneView({ block }: { block: RenderBlock }) {
  const colors = useThemeColors();
  if (block.kind === 'chart') {
    return <ChartView scene={block} colors={colors} />;
  }
  if (block.kind === 'table') {
    return <SceneTable view={block} colors={colors} />;
  }
  return null;
}
