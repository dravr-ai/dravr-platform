// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bottom sheet behind a reply's verdict chip — every claim verdict on that reply, one card each
// ABOUTME: Typed on the shared ClaimVerdict row, so the chip, this sheet and the web drawer cannot disagree

import React from 'react';
import { View, Text, TouchableOpacity, Modal, ScrollView, ActivityIndicator } from 'react-native';
import { Feather } from '@expo/vector-icons';
import type { ClaimVerdict } from '@pierre/shared-types';
import { VERDICT_STATUS_TONE } from '@pierre/shared-types';
import { EVIDENCE_STRENGTH_LABEL_KEY, VERDICT_STATUS_LABEL_KEY } from '@pierre/shared-constants';
import { useThemeColors } from '../../constants/theme';
import { DragIndicator } from '../../components/ui';
import { verdictChipColor, type ThemeColors } from './MessageList';
import { useTranslation } from '@pierre/i18n';

export interface VerdictSheetProps {
  visible: boolean;
  /** Every verdict on the reply whose chip opened the sheet. */
  verdicts: ClaimVerdict[];
  /** The rows are still on their way: the chip landed before the read did. */
  loading: boolean;
  onClose: () => void;
  /** Send a claim back to the coach as a follow-up question. */
  onAskAboutClaim: (verdict: ClaimVerdict) => void;
}

const SECTION_HEADING = 'text-xs font-semibold uppercase tracking-wide text-text-tertiary mt-4 mb-1';

/** Everything the sheet says about one verdict. */
function VerdictCard({
  verdict,
  language,
  colors,
  onAskAboutClaim,
}: {
  verdict: ClaimVerdict;
  language: string;
  colors: ThemeColors;
  onAskAboutClaim: (verdict: ClaimVerdict) => void;
}) {
  const { t } = useTranslation();
  const tint = verdictChipColor(VERDICT_STATUS_TONE[verdict.status], colors);
  const references = (verdict.evidence_refs ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  const emitted = new Date(verdict.created_at);
  const emittedLabel = Number.isNaN(emitted.getTime())
    ? verdict.created_at
    : new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(emitted);

  return (
    <View testID="verdict-card" className="py-4 border-b border-border-default">
      <View className="flex-row flex-wrap gap-2">
        <View className="px-2 py-0.5 rounded-full" style={{ backgroundColor: `${tint}26` }}>
          <Text className="text-xs" style={{ color: tint }}>
            {t(VERDICT_STATUS_LABEL_KEY[verdict.status])}
          </Text>
        </View>
        <View className="px-2 py-0.5 rounded-full bg-background-tertiary">
          <Text className="text-xs text-text-primary">
            {t('chat.evidenceLabel', {
              strength: t(EVIDENCE_STRENGTH_LABEL_KEY[verdict.evidence_strength]),
            })}
          </Text>
        </View>
        <View className="px-2 py-0.5 rounded-full bg-background-tertiary">
          <Text className="text-xs text-text-primary">
            {t('chat.confidenceLabel', { confidence: (verdict.confidence * 100).toFixed(0) })}
          </Text>
        </View>
      </View>

      <Text className={SECTION_HEADING}>{t('chat.theClaim')}</Text>
      <View className="border-l-2 border-primary bg-background-tertiary p-3 rounded-r-lg">
        <Text className="text-sm text-text-primary">{verdict.claim_text}</Text>
      </View>

      {verdict.explanation ? (
        <>
          <Text className={SECTION_HEADING}>{t('chat.detectorFindings')}</Text>
          <Text className="text-sm text-text-primary">{verdict.explanation}</Text>
        </>
      ) : null}

      {references.length > 0 ? (
        <>
          <Text className={SECTION_HEADING}>{t('chat.evidenceReferences')}</Text>
          {references.map((ref) => (
            <Text key={ref} className="text-xs text-text-primary">
              {ref}
            </Text>
          ))}
        </>
      ) : null}

      <Text className="text-xs text-text-tertiary mt-4">
        {t('frag.verdictEmitted')} {emittedLabel}
      </Text>

      <TouchableOpacity
        className="mt-4 py-3 rounded-lg bg-primary items-center"
        onPress={() => onAskAboutClaim(verdict)}
        accessibilityRole="button"
        testID="verdict-ask"
      >
        <Text className="text-sm font-medium text-on-primary">{t('chat.askAboutClaim')}</Text>
      </TouchableOpacity>
    </View>
  );
}

/**
 * Every verdict on one reply, in one sheet.
 *
 * The chip opens it with all the rows of its message — a reply that drew two
 * chips shows two cards, not the first one twice. A chip pressed before the
 * rows landed opens it on the loading line while the host re-reads them.
 */
export function VerdictSheet({ visible, verdicts, loading, onClose, onAskAboutClaim }: VerdictSheetProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();

  return (
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <TouchableOpacity
        className="flex-1 justify-end bg-black/40"
        activeOpacity={1}
        onPress={onClose}
        testID="verdict-sheet-backdrop"
      >
        <View
          className="rounded-t-2xl px-4 pt-3 pb-8 max-h-[85%]"
          style={{ backgroundColor: colors.background.secondary }}
          onStartShouldSetResponder={() => true}
          testID="verdict-sheet"
        >
          <DragIndicator />
          <View className="flex-row items-center justify-between mb-1">
            <Text className="text-lg font-bold text-text-primary" testID="verdict-sheet-title">
              {verdicts.length === 1 ? t('chat.aboutThisClaim') : t('chat.verdictsTitle')}
            </Text>
            <TouchableOpacity onPress={onClose} accessibilityLabel={t('chat.close')} testID="verdict-sheet-close">
              <Feather name="x" size={22} color={colors.text.secondary} />
            </TouchableOpacity>
          </View>

          {loading && verdicts.length === 0 ? (
            <View className="flex-row items-center gap-2 py-6">
              <ActivityIndicator size="small" color={colors.text.secondary} />
              <Text className="text-sm text-text-secondary">{t('chat.verdictsLoading')}</Text>
            </View>
          ) : null}

          <ScrollView keyboardShouldPersistTaps="handled">
            {verdicts.map((verdict) => (
              <VerdictCard
                key={verdict.id}
                verdict={verdict}
                language={language}
                colors={colors}
                onAskAboutClaim={onAskAboutClaim}
              />
            ))}
          </ScrollView>
        </View>
      </TouchableOpacity>
    </Modal>
  );
}
