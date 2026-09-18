// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit test for the "+" follow-through modals — the group-name prompt reads entirely from the catalogue
// ABOUTME: Its question was the one hardcoded English line in an otherwise French dialog (carnet#354)

import React from 'react';
import { render, screen } from '@testing-library/react-native';
import { ChatPlusFlows } from '../ChatPlusFlows';
import type { ChatPlusFlowState } from '../useChatPlusActions';

jest.mock('@pierre/i18n', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

jest.mock('../ConversationParticipantsModal', () => ({
  ConversationParticipantsModal: () => null,
}));

function flows(overrides: Partial<ChatPlusFlowState> = {}): ChatPlusFlowState {
  return {
    conversationId: null,
    groupNamePromptVisible: true,
    participantsVisible: false,
    openParticipants: jest.fn(),
    closeGroupNamePrompt: jest.fn(),
    closeParticipants: jest.fn(),
    submitGroupName: jest.fn(),
    ...overrides,
  };
}

describe('ChatPlusFlows group-name prompt', () => {
  it('draws every line of the dialog from the catalogue, the question included', () => {
    render(<ChatPlusFlows flows={flows()} />);

    expect(screen.getByText('app.newGroupChat')).toBeTruthy();
    expect(screen.getByText('app.groupNamePrompt')).toBeTruthy();
    expect(screen.getByText('app.create')).toBeTruthy();
    expect(screen.getByText('common.cancel')).toBeTruthy();
    expect(screen.queryByText(/group called/)).toBeNull();
  });
});
