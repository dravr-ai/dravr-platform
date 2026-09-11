// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The row of buttons a native header carries on its trailing side
// ABOUTME: One container so every screen spaces its header buttons the same way

import React from 'react';
import { View, type ViewProps } from 'react-native';

export function HeaderActions({ children, ...rest }: ViewProps) {
  return (
    <View {...rest} style={{ flexDirection: 'row', alignItems: 'center', gap: 4, marginRight: -8 }}>
      {children}
    </View>
  );
}
