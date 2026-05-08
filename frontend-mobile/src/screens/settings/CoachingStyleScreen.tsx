// ABOUTME: Coaching-style picker screen — pick output format / cadence preference
// ABOUTME: Persona is orthogonal to the chosen coach — it shapes how every coach speaks

import React, { useEffect, useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import type { CoachingPersona } from '@pierre/shared-types';
import { colors, spacing, glassCard } from '../../constants/theme';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';

interface PersonaOption {
  id: CoachingPersona;
  name: string;
  tagline: string;
  description: string;
  bullets: string[];
}

const PERSONA_OPTIONS: PersonaOption[] = [
  {
    id: 'casual',
    name: 'Casual',
    tagline: 'Friendly, encouraging, no jargon',
    description:
      'Talks to you like a knowledgeable friend texting a quick reply. Round numbers, plain language, short replies.',
    bullets: [
      'Prose, not bullet lists. No structured blocks.',
      'No framework citations or technical acronyms.',
      'Only urgent (P0) push notifications — weekly digest otherwise.',
    ],
  },
  {
    id: 'enthusiast',
    name: 'Enthusiast',
    tagline: 'Prose plus the numbers that matter',
    description:
      'Mostly prose, with a small data block when it actually changes the recommendation. Citations only when you ask why.',
    bullets: [
      'Headline answer first, key datum second. Replies under ~250 words.',
      'Acronyms (CTL, ACWR) glossed on first use, then used freely.',
      'P0 + P1 unsolicited push (acute injury risk, near-overreached).',
    ],
  },
  {
    id: 'power_athlete',
    name: 'Power-athlete',
    tagline: 'Endurance discipline — line-by-line, framework-cited',
    description:
      'Deterministic, auditable output. Per-activity reports with exact numbers. Framework citations on every numeric claim.',
    bullets: [
      'Line-by-line activity blocks with framework labels (Coggan, Banister, Foster).',
      'Full P0–P3 readiness ladder verbatim. 10-point validation checklist.',
      'Full P0/P1/P2 unsolicited push. Pre-workout briefing on request.',
    ],
  },
  {
    id: 'coach',
    name: 'Coach',
    tagline: 'Inherits Power-athlete + roster framing for managing other athletes',
    description:
      'Same Power-athlete discipline, plus per-athlete tenant-scoped reports and roster-wide summaries.',
    bullets: [
      'Athlete-scoped queries — never cross-aggregate the coach and their athletes.',
      'Roster-wide blocks (e.g. ACWR > 1.3 across the squad).',
      'Full P0/P1/P2 push for every athlete; P3 only for the coach themselves.',
    ],
  },
];

const cardStyle: ViewStyle = {
  borderRadius: 16,
  padding: spacing.md,
  marginBottom: spacing.md,
  ...glassCard,
};

const cardSelectedStyle: ViewStyle = {
  borderColor: colors.pierre.violet,
  borderWidth: 1,
};

export function CoachingStyleScreen() {
  const router = useRouter();
  const { user, updateUser } = useAuth();
  const [selected, setSelected] = useState<CoachingPersona>('casual');
  const [isPending, setIsPending] = useState(false);
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  useEffect(() => {
    if (user?.coaching_persona) {
      setSelected(user.coaching_persona);
    }
  }, [user?.coaching_persona]);

  const handleSelect = async (persona: CoachingPersona) => {
    if (persona === selected || isPending) {
      return;
    }
    const previous = selected;
    setSelected(persona);
    setIsPending(true);
    try {
      const result = await userApi.setCoachingPersona(persona);
      setMessage({ type: 'success', text: `Coaching style updated to ${result.persona}.` });
      // Sync the AuthContext user so other screens see the new persona.
      await updateUser({ coaching_persona: result.persona });
    } catch {
      setSelected(previous);
      setMessage({ type: 'error', text: `Failed to update coaching style to ${persona}.` });
    } finally {
      setIsPending(false);
      setTimeout(() => setMessage(null), 3000);
    }
  };

  return (
    <SafeAreaView
      className="flex-1 bg-background-primary"
      testID="coaching-style-screen"
    >
      {/* Header */}
      <View className="flex-row items-center px-4 py-3 border-b border-white/10">
        <TouchableOpacity
          className="p-2 mr-2"
          onPress={() => router.back()}
          testID="back-button"
        >
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary">Coaching style</Text>
      </View>

      <ScrollView
        className="flex-1 px-4"
        contentContainerStyle={{ paddingTop: spacing.md, paddingBottom: spacing.xl }}
        showsVerticalScrollIndicator={false}
      >
        <Text className="text-text-secondary text-sm leading-relaxed mb-4">
          Choose how detailed and structured every coach&rsquo;s replies should be. This is independent of
          which coach you talk to — the same coach speaks differently to a Casual user than to a
          Power-athlete.
        </Text>

        {message && (
          <View
            className={`mb-4 p-3 rounded-lg border ${
              message.type === 'success'
                ? 'bg-emerald-500/10 border-emerald-500/30'
                : 'bg-red-500/10 border-red-500/30'
            }`}
            testID="persona-status"
          >
            <Text
              className={`text-sm ${
                message.type === 'success' ? 'text-emerald-400' : 'text-red-400'
              }`}
            >
              {message.text}
            </Text>
          </View>
        )}

        {PERSONA_OPTIONS.map((option) => {
          const isSelected = selected === option.id;
          return (
            <TouchableOpacity
              key={option.id}
              accessibilityRole="radio"
              accessibilityState={{ selected: isSelected }}
              testID={`persona-card-${option.id}`}
              onPress={() => handleSelect(option.id)}
              disabled={isPending}
              activeOpacity={0.85}
              style={[cardStyle, isSelected ? cardSelectedStyle : null]}
            >
              <View className="flex-row items-center justify-between mb-1.5">
                <Text className="text-base font-semibold text-text-primary">{option.name}</Text>
                {isSelected && (
                  <View className="flex-row items-center">
                    {isPending ? (
                      <ActivityIndicator size="small" color={colors.pierre.violet} />
                    ) : (
                      <Text
                        className="text-xs font-semibold uppercase tracking-wide"
                        style={{ color: colors.pierre.violet }}
                      >
                        Active
                      </Text>
                    )}
                  </View>
                )}
              </View>
              <Text
                className="text-sm mb-2"
                style={{ color: colors.pierre.violet }}
              >
                {option.tagline}
              </Text>
              <Text className="text-sm text-text-secondary leading-relaxed mb-3">
                {option.description}
              </Text>
              <View>
                {option.bullets.map((bullet) => (
                  <View key={bullet} className="flex-row mb-1.5">
                    <Text
                      className="text-sm mr-2 mt-0.5"
                      style={{ color: colors.pierre.violet }}
                    >
                      ›
                    </Text>
                    <Text className="text-xs text-text-tertiary flex-1 leading-relaxed">
                      {bullet}
                    </Text>
                  </View>
                ))}
              </View>
            </TouchableOpacity>
          );
        })}
      </ScrollView>
    </SafeAreaView>
  );
}
