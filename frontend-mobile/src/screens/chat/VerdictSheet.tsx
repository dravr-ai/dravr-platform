// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bottom sheet behind a reply's verdict chip — every claim verdict on that reply, one section each
// ABOUTME: Typed on the shared ClaimVerdict row, so the chip, this sheet and the web drawer cannot disagree

import React from 'react';
import { View, Text, TouchableOpacity, ScrollView, ActivityIndicator, StyleSheet } from 'react-native';
import { Feather } from '@expo/vector-icons';
import type { ClaimVerdict } from '@pierre/shared-types';
import { VERDICT_STATUS_TONE } from '@pierre/shared-types';
import { EVIDENCE_STRENGTH_LABEL_KEY, VERDICT_STATUS_LABEL_KEY } from '@pierre/shared-constants';
import { formatDateTime } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { Sheet } from '../../components/ui';
import { verdictChipPalette, type ThemeColors } from './MessageList';
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

const SECTION_HEADING = 'text-sm font-semibold text-text-secondary mt-4 mb-1';

/**
 * Everything the sheet says about one verdict, as one section under a faint
 * hairline: the status word in the verdict's ink, the evidence and confidence
 * on one tertiary line, the claim quoted behind a hairline rule, then the
 * findings, the references, the timestamp and the inline ask link.
 */
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
  const { ink } = verdictChipPalette(VERDICT_STATUS_TONE[verdict.status], colors);
  const references = (verdict.evidence_refs ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  const emittedLabel = formatDateTime(verdict.created_at, language);
  const meta = [
    t('chat.evidenceLabel', { strength: t(EVIDENCE_STRENGTH_LABEL_KEY[verdict.evidence_strength]) }),
    t('chat.confidenceLabel', { confidence: (verdict.confidence * 100).toFixed(0) }),
  ].join(' · ');

  return (
    <View testID="verdict-card" className="py-4 border-b border-border-faint">
      <Text className="text-sm font-semibold" style={{ color: ink }}>
        {t(VERDICT_STATUS_LABEL_KEY[verdict.status])}
      </Text>
      <Text className="text-sm text-text-tertiary mt-0.5">{meta}</Text>

      <Text className={SECTION_HEADING}>{t('chat.theClaim')}</Text>
      <Text
        className="text-base italic text-text-secondary pl-3"
        style={{ borderLeftWidth: StyleSheet.hairlineWidth, borderLeftColor: colors.border.default }}
      >
        {verdict.claim_text}
      </Text>

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

      <Text
        className="mt-3 text-sm text-primary font-medium"
        onPress={() => onAskAboutClaim(verdict)}
        accessibilityRole="button"
        testID="verdict-ask"
      >
        {t('chat.askAboutClaim')}
      </Text>
    </View>
  );
}

/**
 * Every verdict on one reply, in one sheet.
 *
 * The chip opens it with all the rows of its message — a reply that drew two
 * chips shows two sections, not the first one twice. A chip pressed before the
 * rows landed opens it on the loading line while the host re-reads them.
 */
export function VerdictSheet({ visible, verdicts, loading, onClose, onAskAboutClaim }: VerdictSheetProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();

  return (
    <Sheet visible={visible} onClose={onClose} testID="verdict-sheet" backdropTestID="verdict-sheet-backdrop">
      <View className="flex-row items-center justify-between mb-1">
        <Text className="text-lg font-semibold text-text-primary" testID="verdict-sheet-title">
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
    </Sheet>
  );
}
