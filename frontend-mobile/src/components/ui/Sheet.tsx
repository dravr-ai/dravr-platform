// ABOUTME: The one bottom sheet: a slide-up Modal over the scrim, a 20-radius panel on the secondary ground
// ABOUTME: A tap on the scrim closes it; the panel claims the responder so a tap inside never does (Boreal v2.2 P3.6)

import React, { type ReactNode } from 'react';
import { Modal, TouchableOpacity, View } from 'react-native';
import { useThemeColors } from '../../constants/theme';

export interface SheetProps {
  visible: boolean;
  onClose: () => void;
  /** The sheet's content, laid out from the panel's top edge. */
  children: ReactNode;
  /** Lands on the panel. */
  testID?: string;
  /** Lands on the scrim, the touchable that closes the sheet. */
  backdropTestID?: string;
  /** NativeWind `max-h-*` class for the panel; the default caps it at 85% of the window. */
  maxHeight?: string;
}

/**
 * No drag pill: the panel's rounded top on the scrim is the affordance, and
 * the back gesture (`onRequestClose`) plus the scrim tap are the two ways
 * out. The panel takes `onStartShouldSetResponder` so the scrim's press does
 * not fire for a touch that lands on the content.
 */
export function Sheet({
  visible,
  onClose,
  children,
  testID,
  backdropTestID,
  maxHeight = 'max-h-[85%]',
}: SheetProps) {
  const colors = useThemeColors();

  return (
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <TouchableOpacity
        activeOpacity={1}
        className="flex-1 justify-end bg-scrim/60"
        onPress={onClose}
        testID={backdropTestID}
      >
        <View
          className={`rounded-t-3xl px-4 pt-4 pb-8 ${maxHeight}`}
          style={{ backgroundColor: colors.background.secondary }}
          onStartShouldSetResponder={() => true}
          testID={testID}
        >
          {children}
        </View>
      </TouchableOpacity>
    </Modal>
  );
}
