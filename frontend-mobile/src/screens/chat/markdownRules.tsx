// ABOUTME: Markdown render-rule overrides shared by every chat <Markdown> surface
// ABOUTME: Only `table` is overridden, so a coach table wider than the phone scrolls

import React from 'react';
import { View, ScrollView } from 'react-native';
import { type RenderRules } from 'react-native-markdown-display';

/**
 * Floor width for a table cell, in points. Two short data columns still fit a
 * small phone without scrolling; anything wider scrolls rather than wrapping.
 *
 * Consumed by `buildMarkdownStyles` in MessageList, which owns the themed
 * `th`/`td` styles this width applies to.
 */
export const TABLE_CELL_MIN_WIDTH = 96;

/**
 * Render-rule overrides passed to every `<Markdown>` in the chat surface.
 *
 * The library lays a table out in a plain `View`, so a table wider than the
 * screen is clipped with no way to reach the rest of it. Wrapping it in a
 * horizontal `ScrollView` keeps wide tables — a training week with pace,
 * distance and elevation columns — readable on a phone.
 *
 * Every other node kind falls through to the library's own rules; overriding
 * `table` alone deliberately leaves headings, lists and links untouched.
 */
export const MARKDOWN_RULES: RenderRules = {
  table: (node, children, _parent, styles) => (
    <ScrollView
      key={node.key}
      testID="markdown-table-scroll"
      horizontal
      showsHorizontalScrollIndicator={false}
      // The table's own border and margins live on the inner View, so the
      // scroll container wraps its content exactly rather than stretching.
      contentContainerStyle={{ flexGrow: 0 }}
    >
      <View style={styles._VIEW_SAFE_table}>{children}</View>
    </ScrollView>
  ),
};
