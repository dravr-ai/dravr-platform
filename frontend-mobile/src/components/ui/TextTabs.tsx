// ABOUTME: Text tabs — one horizontal row of 13 / 500 labels 20 apart over a hairline, the active one in the primary ink with a 2 px primary underline
// ABOUTME: The filter language of every athlete surface: the kind filter on Memory, the pickers in Notification preferences, Discover's categories

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Pressable, ScrollView, StyleSheet, Text, View, type LayoutChangeEvent } from 'react-native';
import { ScrollFadeContainer } from './ScrollFadeContainer';

export interface TextTabItem {
  key: string;
  label: string;
  /** A count drawn beside the label in mono, as a section label carries its own. */
  count?: number;
}

export interface TextTabsProps {
  items: TextTabItem[];
  /** The `key` of the selected item. */
  value: string;
  onChange: (key: string) => void;
  /** Lands on the row; each tab gets `${testID}-${item.key}`. */
  testID?: string;
  /** Extra classes on the wrapping view. */
  className?: string;
}

/** The row's side inset (`px-4`): a tab scrolled into view keeps it. */
const EDGE_INSET = 16;

/**
 * The web's underline `Tabs` on React Native. The row scrolls sideways when
 * the words outrun the phone instead of wrapping, and every tab carries an
 * underline of the same height — the inactive ones transparent — so the row
 * does not jump when the selection moves.
 *
 * A row that scrolls says so: the edge it cuts fades into the page. The
 * selected tab is scrolled into view when the row is drawn and whenever the
 * selection moves, so a filter chosen before the screen opened (a deep link
 * to one athlete) is never off the edge with nothing on screen naming it.
 */
export function TextTabs({ items, value, onChange, testID, className }: TextTabsProps) {
  const scrollRef = useRef<ScrollView>(null);
  const frames = useRef(new Map<string, { x: number; width: number }>());
  const [rowWidth, setRowWidth] = useState(0);
  const offset = useRef(0);
  const [selectedFrame, setSelectedFrame] = useState<{ x: number; width: number } | null>(null);

  const onTabLayout = useCallback(
    (key: string, event: LayoutChangeEvent) => {
      const { x, width } = event.nativeEvent.layout;
      frames.current.set(key, { x, width });
      if (key === value) setSelectedFrame({ x, width });
    },
    [value],
  );

  useEffect(() => {
    setSelectedFrame(frames.current.get(value) ?? null);
  }, [value]);

  // Once the selected tab and the row are measured, bring the tab in: its
  // right edge inside the row, then its left edge, keeping the side inset.
  useEffect(() => {
    if (selectedFrame === null || rowWidth === 0) return;
    const { x, width } = selectedFrame;
    const current = offset.current;
    // Written as min/max rather than comparisons: the untranslated-string
    // scanner reads a `>` … `<` pair on one line as JSX text.
    const showsRightEdge = Math.max(current, x + width + EDGE_INSET - rowWidth);
    const next = Math.min(showsRightEdge, Math.max(0, x - EDGE_INSET));
    if (next !== current) {
      offset.current = next;
      scrollRef.current?.scrollTo({ x: next, animated: false });
    }
  }, [selectedFrame, rowWidth]);

  return (
    <View
      className={`border-b border-border ${className ?? ''}`}
      style={{ borderBottomWidth: StyleSheet.hairlineWidth }}
      onLayout={(event) => setRowWidth(event.nativeEvent.layout.width)}
      testID={testID}
    >
      {/* The edge a scrolling row cuts fades into the page, so the row says it scrolls. */}
      <ScrollFadeContainer
        scrollRef={scrollRef}
        onScrollX={(x) => {
          offset.current = x;
        }}
        contentContainerClassName="flex-row gap-5 px-4"
        testID={testID ? `${testID}-row` : undefined}
      >
        {items.map((item) => {
          const selected = item.key === value;
          return (
            <Pressable
              key={item.key}
              className="py-2.5"
              onPress={() => onChange(item.key)}
              onLayout={(event) => onTabLayout(item.key, event)}
              accessibilityRole="tab"
              accessibilityState={{ selected }}
              testID={testID ? `${testID}-${item.key}` : undefined}
            >
              <View className="flex-row items-baseline gap-1.5">
                <Text className={`text-sm font-medium ${selected ? 'text-text-primary' : 'text-text-secondary'}`}>
                  {item.label}
                </Text>
                {item.count !== undefined && (
                  <Text
                    className="font-mono text-xs text-text-secondary"
                    testID={testID ? `${testID}-${item.key}-count` : undefined}
                  >
                    {item.count}
                  </Text>
                )}
              </View>
              <View className={`h-0.5 mt-1.5 ${selected ? 'bg-primary' : 'bg-transparent'}`} />
            </Pressable>
          );
        })}
      </ScrollFadeContainer>
    </View>
  );
}
