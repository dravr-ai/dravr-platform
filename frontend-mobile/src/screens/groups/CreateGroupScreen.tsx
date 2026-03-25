// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Create group screen for building a new coaching group
// ABOUTME: Form with name, description, coach selection, and max members picker

import React, { useState, useCallback, useEffect } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { useFocusEffect } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { colors, spacing, glassCard } from '../../constants/theme';
import { Input } from '../../components/ui/Input';
import { Button } from '../../components/ui/Button';
import { coachesApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useGroupActions } from '../../hooks/useGroups';
import type { Coach } from '../../types';

const MAX_MEMBERS_OPTIONS = [5, 10, 15, 20, 25, 50];

const coachItemStyle: ViewStyle = {
  ...glassCard,
  borderRadius: 12,
  borderColor: 'rgba(139, 92, 246, 0.15)',
};

const selectedCoachStyle: ViewStyle = {
  ...glassCard,
  borderRadius: 12,
  borderColor: colors.pierre.violet,
  borderWidth: 2,
};

export function CreateGroupScreen() {
  const router = useRouter();
  const { isAuthenticated } = useAuth();
  const { createGroup, isCreating, createError } = useGroupActions();

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [selectedCoachId, setSelectedCoachId] = useState<string | null>(null);
  const [maxMembers, setMaxMembers] = useState(20);
  const [coaches, setCoaches] = useState<Coach[]>([]);
  const [isLoadingCoaches, setIsLoadingCoaches] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Load available coaches
  const loadCoaches = useCallback(async () => {
    if (!isAuthenticated) return;
    try {
      setIsLoadingCoaches(true);
      const response = await coachesApi.listCoaches();
      setCoaches(response.coaches);
      // Auto-select first coach if only one available
      if (response.coaches.length === 1) {
        setSelectedCoachId(response.coaches[0].id);
      }
    } catch (err) {
      console.error('Failed to load coaches:', err);
    } finally {
      setIsLoadingCoaches(false);
    }
  }, [isAuthenticated]);

  useFocusEffect(
    useCallback(() => {
      loadCoaches();
    }, [loadCoaches]),
  );

  // Propagate mutation errors
  useEffect(() => {
    if (createError) {
      setErrorMessage(
        createError instanceof Error ? createError.message : 'Failed to create group',
      );
    }
  }, [createError]);

  const handleCreate = useCallback(async () => {
    const trimmedName = name.trim();
    if (!trimmedName) {
      setErrorMessage('Group name is required');
      return;
    }
    if (!selectedCoachId) {
      setErrorMessage('Please select a coach');
      return;
    }

    setErrorMessage(null);

    try {
      await createGroup({
        name: trimmedName,
        description: description.trim() || undefined,
        coach_id: selectedCoachId,
        max_members: maxMembers,
      });
      router.back();
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to create group';
      setErrorMessage(msg);
    }
  }, [name, description, selectedCoachId, maxMembers, createGroup, router]);

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="create-group-screen">
      {/* Header */}
      <View className="flex-row items-center px-4 py-3 border-b border-border-subtle">
        <TouchableOpacity className="p-2" onPress={() => router.back()} testID="back-button">
          <Feather name="arrow-left" size={22} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary text-center mr-10">
          Create Group
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
          {/* Group Name */}
          <Input
            label="Group Name"
            placeholder="e.g. Marathon Training Squad"
            value={name}
            onChangeText={(text) => {
              setName(text);
              setErrorMessage(null);
            }}
            maxLength={100}
            testID="group-name-input"
          />

          {/* Description */}
          <Input
            label="Description (optional)"
            placeholder="What is this group about?"
            value={description}
            onChangeText={setDescription}
            multiline
            numberOfLines={3}
            maxLength={500}
            testID="group-description-input"
          />

          {/* Coach Selector */}
          <Text className="text-text-secondary text-sm mb-2 font-medium">
            Select Coach
          </Text>
          {isLoadingCoaches ? (
            <View className="py-6 items-center">
              <ActivityIndicator size="small" color={colors.pierre.violet} />
              <Text className="text-text-tertiary text-sm mt-2">Loading coaches...</Text>
            </View>
          ) : coaches.length === 0 ? (
            <View className="py-6 items-center">
              <Feather name="alert-circle" size={24} color={colors.text.secondary} />
              <Text className="text-text-secondary text-sm mt-2">
                No coaches available. Install a coach first.
              </Text>
            </View>
          ) : (
            <View className="mb-4">
              {coaches.map((coach) => {
                const isSelected = selectedCoachId === coach.id;
                return (
                  <TouchableOpacity
                    key={coach.id}
                    className="flex-row items-center p-3 mb-2"
                    style={isSelected ? selectedCoachStyle : coachItemStyle}
                    onPress={() => {
                      setSelectedCoachId(coach.id);
                      setErrorMessage(null);
                    }}
                    testID={`coach-option-${coach.id}`}
                  >
                    <View className="flex-1">
                      <Text
                        className="text-base font-semibold text-text-primary"
                        numberOfLines={1}
                      >
                        {coach.title}
                      </Text>
                      {coach.category && (
                        <Text className="text-xs text-text-tertiary capitalize mt-0.5">
                          {coach.category}
                        </Text>
                      )}
                    </View>
                    {isSelected && (
                      <Feather name="check-circle" size={20} color={colors.pierre.violet} />
                    )}
                  </TouchableOpacity>
                );
              })}
            </View>
          )}

          {/* Max Members Picker */}
          <Text className="text-text-secondary text-sm mb-2 font-medium">
            Max Members
          </Text>
          <View className="flex-row flex-wrap gap-2 mb-6">
            {MAX_MEMBERS_OPTIONS.map((count) => {
              const isSelected = maxMembers === count;
              return (
                <TouchableOpacity
                  key={count}
                  className={`px-4 py-2 rounded-lg border ${
                    isSelected
                      ? 'bg-pierre-violet/20 border-pierre-violet'
                      : 'bg-background-secondary border-border-default'
                  }`}
                  onPress={() => setMaxMembers(count)}
                  testID={`max-members-${count}`}
                >
                  <Text
                    className={`text-sm font-medium ${
                      isSelected ? 'text-pierre-violet' : 'text-text-secondary'
                    }`}
                  >
                    {count}
                  </Text>
                </TouchableOpacity>
              );
            })}
          </View>

          {/* Error Display */}
          {errorMessage && (
            <View className="p-3 bg-error/10 border border-error/30 rounded-lg mb-4">
              <Text className="text-error text-sm text-center">{errorMessage}</Text>
            </View>
          )}

          {/* Create Button */}
          <Button
            title={isCreating ? 'Creating...' : 'Create Group'}
            onPress={handleCreate}
            variant="primary"
            size="lg"
            fullWidth
            loading={isCreating}
            disabled={!name.trim() || !selectedCoachId}
            testID="create-group-button"
          />
        </ScrollView>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}
