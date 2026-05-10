// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Join group screen for entering an invite code to join a coaching group
// ABOUTME: Supports manual code entry and deep link params, with group preview and consent

import React, { useState, useCallback, useEffect } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  KeyboardAvoidingView,
  Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter, useLocalSearchParams } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { spacing, glassCard, useThemeColors } from '../../constants/theme';
import { Input } from '../../components/ui/Input';
import { Button } from '../../components/ui/Button';
import { useGroupActions } from '../../hooks/useGroups';

export function JoinGroupScreen() {
  const colors = useThemeColors();
  const router = useRouter();
  const params = useLocalSearchParams<{ code?: string }>();
  const { joinGroup, isJoining, joinError } = useGroupActions();

  const [inviteCode, setInviteCode] = useState(params.code ?? '');
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Populate from deep link params
  useEffect(() => {
    if (params.code) {
      setInviteCode(params.code);
    }
  }, [params.code]);

  // Propagate mutation errors
  useEffect(() => {
    if (joinError) {
      setErrorMessage(
        joinError instanceof Error ? joinError.message : 'Failed to join group',
      );
    }
  }, [joinError]);

  const handleJoin = useCallback(async () => {
    const trimmedCode = inviteCode.trim();
    if (!trimmedCode) {
      setErrorMessage('Please enter an invite code');
      return;
    }

    setErrorMessage(null);
    setSuccessMessage(null);

    try {
      await joinGroup({ invite_code: trimmedCode });
      setSuccessMessage('You have joined the group!');
      // Navigate back to group list after a brief delay
      setTimeout(() => {
        router.back();
      }, 1500);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to join group';
      setErrorMessage(msg);
    }
  }, [inviteCode, joinGroup, router]);

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="join-group-screen">
      {/* Header */}
      <View className="flex-row items-center px-4 py-3 border-b border-border-subtle">
        <TouchableOpacity className="p-2" onPress={() => router.back()} testID="back-button">
          <Feather name="arrow-left" size={22} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary text-center mr-10">
          Join Group
        </Text>
      </View>

      <KeyboardAvoidingView
        className="flex-1"
        behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
      >
        <ScrollView
          className="flex-1"
          contentContainerStyle={{ padding: spacing.lg }}
          keyboardShouldPersistTaps="handled"
        >
          {/* Icon */}
          <View className="items-center mb-6">
            <View
              className="w-20 h-20 rounded-full items-center justify-center"
              style={{
                backgroundColor: 'rgba(139, 92, 246, 0.1)',
                shadowColor: colors.pierre.violet,
                shadowOffset: { width: 0, height: 0 },
                shadowOpacity: 0.3,
                shadowRadius: 20,
              }}
            >
              <Feather name="log-in" size={36} color={colors.pierre.violet} />
            </View>
          </View>

          {/* Instructions */}
          <Text className="text-text-primary text-lg font-bold text-center mb-2">
            Enter Invite Code
          </Text>
          <Text className="text-text-secondary text-base text-center mb-6">
            Ask your coach or group admin for the invite code to join their coaching group.
          </Text>

          {/* Invite Code Input */}
          <Input
            label="Invite Code"
            placeholder="e.g. ABC123XY"
            value={inviteCode}
            onChangeText={(text) => {
              setInviteCode(text);
              setErrorMessage(null);
              setSuccessMessage(null);
            }}
            autoCapitalize="characters"
            autoCorrect={false}
            testID="invite-code-input"
          />

          {/* Consent Text */}
          <View
            className="p-3 rounded-lg mb-6"
            style={{
              backgroundColor: glassCard.background,
              borderWidth: 1,
              borderColor: 'rgba(139, 92, 246, 0.1)',
              borderRadius: 8,
            }}
          >
            <Text className="text-text-tertiary text-xs leading-4">
              By joining a group, your coach will have access to your training data
              for coaching purposes. You can control peer data sharing after joining
              through group settings.
            </Text>
          </View>

          {/* Error Display */}
          {errorMessage && (
            <View className="p-3 bg-error/10 border border-error/30 rounded-lg mb-4">
              <Text className="text-error text-sm text-center">{errorMessage}</Text>
            </View>
          )}

          {/* Success Display */}
          {successMessage && (
            <View className="p-3 bg-pierre-activity/10 border border-pierre-activity/30 rounded-lg mb-4">
              <View className="flex-row items-center justify-center gap-2">
                <Feather name="check-circle" size={18} color={colors.pierre.activity} />
                <Text className="text-sm font-semibold" style={{ color: colors.pierre.activity }}>
                  {successMessage}
                </Text>
              </View>
            </View>
          )}

          {/* Join Button */}
          <Button
            title={isJoining ? 'Joining...' : 'Join Group'}
            onPress={handleJoin}
            variant="primary"
            size="lg"
            fullWidth
            loading={isJoining}
            disabled={!inviteCode.trim()}
            testID="join-group-button"
          />
        </ScrollView>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}
