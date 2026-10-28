// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Discover row's install count reads in the athlete's language, its form chosen by countKey
// ABOUTME: carnet#1212: an inline `=== 1` read « 0 utilisateurs » for a listing nobody had installed

import React from 'react';
import { act, render, screen } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { DiscoverRow } from '../src/screens/store/DiscoverRow';
import type { AgentCategory, StoreAgent } from '../src/types';

function agent(install_count: number): StoreAgent {
  return {
    id: 'store-coach-1',
    title: 'Test Store Coach',
    description: 'A published coach for the store',
    category: 'training' as AgentCategory,
    tags: [],
    sample_prompts: [],
    token_count: 800,
    install_count,
    icon_url: null,
    published_at: '2024-01-15T00:00:00Z',
    author_id: null,
  };
}

function countFor(installs: number): unknown {
  const { unmount } = render(<DiscoverRow agent={agent(installs)} onPress={jest.fn()} index={0} />);
  const children = screen.getByTestId('install-count').props.children;
  unmount();
  return children;
}

describe('DiscoverRow install count', () => {
  afterEach(async () => {
    await act(async () => {
      await i18n.changeLanguage('en');
    });
  });

  it('reads 0 and 1 in the French singular and 2 in the plural', async () => {
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    expect(countFor(0)).toBe('0 utilisateur');
    expect(countFor(1)).toBe('1 utilisateur');
    expect(countFor(2)).toBe('2 utilisateurs');
  });

  it('keeps 0 plural in English', () => {
    expect(countFor(0)).toBe('0 users');
    expect(countFor(1)).toBe('1 user');
  });
});
