// ABOUTME: Text tabs — one horizontal row of 13 / 500 labels 20 apart over a hairline, the active one in the primary ink with a 2 px primary underline
// ABOUTME: The filter language of every athlete surface: the kind filter on Memory, the pickers in Notification preferences, Discover's categories

import React from 'react';
import { Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

export interface TextTabItem {
  key: string;
  label: string;
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

/**
 * The web's underline `Tabs` on React Native. The row scrolls sideways when
 * the words outrun the phone instead of wrapping, and every tab carries an
 * underline of the same height — the inactive ones transparent — so the row
 * does not jump when the selection moves.
 */
export function TextTabs({ items, value, onChange, testID, className }: TextTabsProps) {
  return (
    <View
      className={`border-b border-border ${className ?? ''}`}
      style={{ borderBottomWidth: StyleSheet.hairlineWidth }}
      testID={testID}
    >
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerClassName="flex-row gap-5 px-4"
      >
        {items.map((item) => {
          const selected = item.key === value;
          return (
            <Pressable
              key={item.key}
              className="py-2.5"
              onPress={() => onChange(item.key)}
              accessibilityRole="tab"
              accessibilityState={{ selected }}
              testID={testID ? `${testID}-${item.key}` : undefined}
            >
              <Text className={`text-sm font-medium ${selected ? 'text-text-primary' : 'text-text-secondary'}`}>
                {item.label}
              </Text>
              <View className={`h-0.5 mt-1.5 ${selected ? 'bg-primary' : 'bg-transparent'}`} />
            </Pressable>
          );
        })}
      </ScrollView>
    </View>
  );
}
