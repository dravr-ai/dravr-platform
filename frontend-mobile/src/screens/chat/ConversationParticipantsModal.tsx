// ABOUTME: Who is in the open conversation — list, add by user id, remove a member
// ABOUTME: The mobile caller of the participants routes; the owner is listed but never removable

import React, { useCallback, useEffect, useState } from 'react';
import { View, Text, TouchableOpacity, Modal, FlatList, ActivityIndicator } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import type { ConversationParticipant } from '@pierre/shared-types';
import { Input } from '../../components/ui';
import { chatApi } from '../../services/api';
import { extractErrorMessage } from '../../utils/errorMessages';
import { useThemeColors } from '../../constants/theme';
import { useTranslation } from '@pierre/i18n';

interface ConversationParticipantsModalProps {
  visible: boolean;
  conversationId: string | null;
  onClose: () => void;
}

/**
 * The participants sheet behind the title menu's "Participants" item.
 *
 * Lists everyone in the thread and lets any participant add a tenant member
 * by user id or remove a member. Refusals come back from the server (a
 * non-member of the tenant is 403, the owner cannot be removed) and are shown
 * inline rather than guessed at client-side.
 */
export function ConversationParticipantsModal({
  visible,
  conversationId,
  onClose,
}: ConversationParticipantsModalProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const [participants, setParticipants] = useState<ConversationParticipant[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newUserId, setNewUserId] = useState('');

  const load = useCallback(async () => {
    if (!conversationId) return;
    setIsLoading(true);
    setError(null);
    try {
      setParticipants(await chatApi.listParticipants(conversationId));
    } catch (err) {
      setError(extractErrorMessage(err, 'Failed to load participants'));
    } finally {
      setIsLoading(false);
    }
  }, [conversationId]);

  useEffect(() => {
    if (visible) {
      void load();
    }
  }, [visible, load]);

  const handleAdd = useCallback(async () => {
    const trimmed = newUserId.trim();
    if (!conversationId || !trimmed) return;
    setIsSaving(true);
    setError(null);
    try {
      await chatApi.addParticipant(conversationId, trimmed);
      setNewUserId('');
      await load();
    } catch (err) {
      setError(extractErrorMessage(err, 'Failed to add participant'));
    } finally {
      setIsSaving(false);
    }
  }, [conversationId, newUserId, load]);

  const handleRemove = useCallback(
    async (userId: string) => {
      if (!conversationId) return;
      setIsSaving(true);
      setError(null);
      try {
        await chatApi.removeParticipant(conversationId, userId);
        await load();
      } catch (err) {
        setError(extractErrorMessage(err, 'Failed to remove participant'));
      } finally {
        setIsSaving(false);
      }
    },
    [conversationId, load],
  );

  return (
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <View className="flex-1 justify-end bg-black/40">
        <View
          className="rounded-t-2xl px-4 pt-4 pb-8"
          style={{ backgroundColor: colors.background.secondary, maxHeight: '75%' }}
          testID="conversation-participants-modal"
        >
          <View className="flex-row items-center justify-between mb-3">
            <Text className="text-lg font-semibold text-text-primary">{t('app.participants')}</Text>
            <TouchableOpacity onPress={onClose} testID="participants-close" accessibilityLabel={t('common.close')}>
              <Ionicons name="close" size={22} color={colors.text.secondary} />
            </TouchableOpacity>
          </View>

          {isLoading ? (
            <ActivityIndicator testID="participants-loading" />
          ) : (
            <FlatList
              data={participants}
              keyExtractor={p => p.user_id}
              renderItem={({ item }) => (
                <View className="flex-row items-center justify-between py-2 border-b border-border-subtle">
                  <View className="flex-1 mr-2">
                    <Text className="text-sm text-text-primary" numberOfLines={1} testID={`participant-${item.user_id}`}>
                      {item.user_id}
                    </Text>
                    <Text className="text-xs text-text-tertiary">{item.role}</Text>
                  </View>
                  {item.role !== 'owner' && (
                    <TouchableOpacity
                      onPress={() => handleRemove(item.user_id)}
                      disabled={isSaving}
                      accessibilityLabel={`Remove ${item.user_id}`}
                      testID={`remove-${item.user_id}`}
                    >
                      <Ionicons name="person-remove-outline" size={20} color={colors.error} />
                    </TouchableOpacity>
                  )}
                </View>
              )}
            />
          )}

          <View className="flex-row items-center mt-4">
            <Input
              containerStyle={{ flex: 1, marginRight: 8 }}
              value={newUserId}
              onChangeText={setNewUserId}
              placeholder={t('app.userIdToAdd')}
              autoCapitalize="none"
              autoCorrect={false}
              testID="participant-user-id-input"
            />
            <TouchableOpacity
              className="px-3 py-2 rounded-lg bg-primary"
              onPress={handleAdd}
              disabled={isSaving || newUserId.trim() === ''}
              accessibilityLabel={t('app.addParticipant')}
              testID="participant-add-button"
            >
              <Text className="text-sm font-medium text-white">{t('app.add')}</Text>
            </TouchableOpacity>
          </View>

          {error && (
            <Text className="text-sm text-error mt-3" testID="participants-error">
              {error}
            </Text>
          )}
        </View>
      </View>
    </Modal>
  );
}
