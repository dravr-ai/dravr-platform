// ABOUTME: A group on a settings screen — a 13 / 600 title, one optional line under it, an optional action, then its content
// ABOUTME: The header insets itself 16; the content is full-bleed so rows and empty states, which pay their own inset, sit flush (DESIGN.md §10)

import React, { Children, type ReactNode } from 'react';
import { Text, View } from 'react-native';

export interface SectionProps {
  /** The group's name, in the system face at 13 / 600 — not a display heading. */
  title: string;
  /** One line under the title saying what the group is for. */
  description?: string;
  /** A control that belongs to the whole group, right of the title: an "Add" link, a count. */
  actions?: ReactNode;
  /** The rows, fields or empty state under the header; a header alone is a valid group. */
  children?: ReactNode;
  /** Extra classes on the wrapping view. */
  className?: string;
  testID?: string;
}

/**
 * The web's `Section` on React Native. No fill, no border, no radius: the
 * title and its line, then the rows or fields 12 below. The space between two
 * sections is paid by the parent — `gap-8` (32) on the column that stacks
 * them — so a lone section carries no margin of its own.
 *
 * The header pays the pane's 16 inset itself and the content slot pays none:
 * a `Row`, an `EmptyState` or a `TextTabs` carries its own `px-4`, so its
 * hairline insets to the text while its press target runs to the pane's edge,
 * and title and row text share one left edge without either cancelling the
 * other. Content that pays no inset of its own — a plain `Text`, a field, a
 * button — adds `px-4` itself. The slot is not drawn when there is nothing in
 * it, so a group that is only a header and its action pays no 12 of nothing.
 */
export function Section({ title, description, actions, children, className, testID }: SectionProps) {
  // `toArray` drops null, undefined and booleans, so `{cond && <X />}` with a
  // false condition reads as no content, as does an empty list.
  const hasContent = Children.toArray(children).length > 0;

  return (
    <View className={`min-w-0 ${className ?? ''}`} testID={testID}>
      <View className="flex-row items-start justify-between gap-4 px-4">
        <View className="flex-1 min-w-0">
          <Text className="text-sm font-semibold text-text-primary">{title}</Text>
          {description !== undefined && (
            <Text className="text-sm text-text-secondary mt-0.5">{description}</Text>
          )}
        </View>
        {actions !== undefined && (
          <View className="flex-row shrink-0 items-center gap-1.5">{actions}</View>
        )}
      </View>
      {hasContent && <View className="mt-3">{children}</View>}
    </View>
  );
}
