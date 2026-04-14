// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Category filter button for coaches panel
// ABOUTME: Memoized for performance when rendering multiple category filters

import { memo } from 'react';
import { clsx } from 'clsx';
import { getCategoryIcon } from './utils';

interface CategoryFilterButtonProps {
  category: string | null;
  label: string;
  isSelected: boolean;
  onClick: () => void;
  showIcon?: boolean;
}

const CategoryFilterButton = memo(function CategoryFilterButton({
  category,
  label,
  isSelected,
  onClick,
  showIcon = true,
}: CategoryFilterButtonProps) {
  return (
    <button
      onClick={onClick}
      className={clsx(
        'px-4 py-2 text-sm font-medium rounded-full whitespace-nowrap transition-colors flex items-center gap-1.5',
        isSelected
          ? 'bg-pierre-violet text-on-surface shadow-ambient'
          : 'bg-surface-container-low text-on-surface-variant hover:bg-surface-container hover:text-on-surface'
      )}
    >
      {showIcon && category && <span>{getCategoryIcon(category)}</span>}
      {label}
    </button>
  );
});

export default CategoryFilterButton;
