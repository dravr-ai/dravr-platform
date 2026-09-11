// ABOUTME: The expo-router mock every unit test shares — routing hooks, and a Stack.Screen that renders its header parts
// ABOUTME: A test that overrides a hook builds its mock from here, so a screen's native header stays observable

/* eslint-env node, jest */

/**
 * Build the `expo-router` mock.
 *
 * The screens configure their native header through `<Stack.Screen options>`
 * — a title view, header buttons, the system search field. None of that is a
 * React subtree in production (the native bar draws it), so the mock renders
 * what a test can observe: the `headerLeft`, `headerTitle` and `headerRight`
 * components under `stack-screen-header`, a string title as
 * `stack-header-title`, and the search field as a plain input under
 * `header-search-input` whose text reaches `onChangeText` the way the native
 * event does.
 *
 * `overrides` replaces any member, so a test can hand in its own router.
 */
function createExpoRouterMock(overrides = {}) {
  const React = require('react');
  const { Text, TextInput, View } = require('react-native');

  function StackScreen({ options }) {
    if (!options) return null;
    const parts = [];
    if (typeof options.headerLeft === 'function') {
      parts.push(
        React.createElement(
          View,
          { key: 'left', testID: 'stack-header-left' },
          options.headerLeft({ canGoBack: true }),
        ),
      );
    }
    if (typeof options.headerTitle === 'function') {
      parts.push(
        React.createElement(
          View,
          { key: 'title', testID: 'stack-header-title-view' },
          options.headerTitle({ children: options.title ?? '' }),
        ),
      );
    } else if (typeof options.title === 'string' && options.title.length > 0) {
      parts.push(
        React.createElement(Text, { key: 'title', testID: 'stack-header-title' }, options.title),
      );
    }
    if (typeof options.headerRight === 'function') {
      parts.push(
        React.createElement(
          View,
          { key: 'right', testID: 'stack-header-right' },
          options.headerRight({ canGoBack: true }),
        ),
      );
    }
    const search = options.headerSearchBarOptions;
    if (search) {
      parts.push(
        React.createElement(TextInput, {
          key: 'search',
          testID: 'header-search-input',
          placeholder: search.placeholder,
          onChangeText: (text) => search.onChangeText?.({ nativeEvent: { text } }),
          onSubmitEditing: () => search.onSearchButtonPress?.({ nativeEvent: { text: '' } }),
        }),
      );
    }
    if (parts.length === 0) return null;
    return React.createElement(View, { testID: 'stack-screen-header' }, parts);
  }

  // One function per navigator: `Object.assign` mutates its target, so a
  // shared passthrough would give Stack and Tabs one `Screen` between them.
  const passthrough = ({ children }) => React.createElement(View, null, children);
  const StackNavigator = ({ children }) => React.createElement(View, null, children);
  const TabsNavigator = ({ children }) => React.createElement(View, null, children);

  return {
    useRouter: () => ({
      push: jest.fn(),
      replace: jest.fn(),
      back: jest.fn(),
      navigate: jest.fn(),
      canGoBack: () => true,
    }),
    useLocalSearchParams: () => ({}),
    useGlobalSearchParams: () => ({}),
    useSegments: () => [],
    usePathname: () => '/',
    useFocusEffect: (cb) => {
      React.useEffect(() => cb(), [cb]);
    },
    Link: ({ children }) => children,
    Redirect: () => null,
    Slot: passthrough,
    Stack: Object.assign(StackNavigator, { Screen: StackScreen }),
    Tabs: Object.assign(TabsNavigator, { Screen: () => null }),
    ...overrides,
  };
}

/**
 * Build the `expo-router/unstable-native-tabs` mock: the tab bar as a row of
 * labelled items carrying the test id the native item would, so a test can
 * pin the tab set and the unread badge.
 */
function createNativeTabsMock() {
  const React = require('react');
  const { Text, View } = require('react-native');

  const Trigger = ({ name, unstable_nativeProps, children }) =>
    React.createElement(
      View,
      { testID: unstable_nativeProps?.tabBarItemTestID ?? `tab-${name}`, accessibilityRole: 'tab' },
      children,
    );
  Trigger.Label = ({ children }) => React.createElement(Text, { testID: 'tab-label' }, children);
  Trigger.Icon = () => null;
  Trigger.Badge = ({ children, hidden }) =>
    hidden ? null : React.createElement(Text, { testID: 'tab-badge' }, children);

  const NativeTabs = ({ children }) => React.createElement(View, { testID: 'native-tabs' }, children);
  NativeTabs.Trigger = Trigger;

  return { NativeTabs };
}

module.exports = { createExpoRouterMock, createNativeTabsMock };
