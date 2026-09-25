// ABOUTME: Shared base ESLint configuration for Pierre frontend applications
// ABOUTME: Enforces consistent TypeScript and code quality standards

/**
 * Base TypeScript rules shared across all Pierre frontend applications.
 * These rules enforce strict type safety and consistent code quality.
 */
export const baseTypeScriptRules = {
  // CRITICAL: Enforce no explicit any - use unknown with type guards instead
  '@typescript-eslint/no-explicit-any': 'error',

  // Unused variables are errors, except those prefixed with _
  '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],

  // Allow inference for return types (explicit is optional)
  '@typescript-eslint/explicit-function-return-type': 'off',
  '@typescript-eslint/explicit-module-boundary-types': 'off',

  // Disable base rule in favor of TypeScript version
  'no-unused-vars': 'off',
};

/**
 * Base React rules shared across web and mobile applications.
 */
export const baseReactRules = {
  // React 17+ doesn't need React in scope
  'react/react-in-jsx-scope': 'off',

  // We use TypeScript for prop validation
  'react/prop-types': 'off',
};

/**
 * React Hooks rules - critical for correctness
 */
export const reactHooksRules = {
  'react-hooks/rules-of-hooks': 'error',
  'react-hooks/exhaustive-deps': 'warn',
};

const RAW_ERROR_MESSAGE =
  "Render a failed call with describeApiError(err, { t, fallbackKey }) from @pierre/ui-logic. A thrown error's own message is axios's English prose ('Request failed with status code 500', 'Network Error'), and it reaches the athlete untranslated.";

/**
 * `no-restricted-syntax` entries that refuse reading a thrown error's own text
 * as the sentence to show.
 *
 * `err instanceof Error ? err.message : fallback` looks like a safe fallback,
 * but for a failed request the message is axios's, in English, whatever
 * language the interface is in. The shared classifier reads status and
 * transport instead and answers from the catalogue, so the raw read is refused
 * here rather than caught site by site in review. The three shapes are the
 * ternary, the ternary guarded by a truthiness check, and the `if` that
 * assigns the message to what the screen shows.
 *
 * Exported as entries rather than as a rule setting because
 * `no-restricted-syntax` takes one list per file: a client that restricts
 * other syntax spreads these into the same list.
 */
export const rawErrorMessageRestrictions = [
  {
    selector:
      "ConditionalExpression[test.operator='instanceof'][test.right.name='Error'][consequent.property.name='message']",
    message: RAW_ERROR_MESSAGE,
  },
  {
    selector:
      "ConditionalExpression[test.type='LogicalExpression'][test.left.operator='instanceof'][test.left.right.name='Error'][consequent.property.name='message']",
    message: RAW_ERROR_MESSAGE,
  },
  {
    selector:
      "IfStatement[test.operator='instanceof'][test.right.name='Error'] AssignmentExpression[right.property.name='message']",
    message: RAW_ERROR_MESSAGE,
  },
];

/**
 * Relaxed rules for test files
 */
export const testFileRules = {
  '@typescript-eslint/no-unused-vars': 'off',
  '@typescript-eslint/no-explicit-any': 'off',
  'react-hooks/exhaustive-deps': 'off',
  'no-unused-vars': 'off',
};

export default {
  baseTypeScriptRules,
  baseReactRules,
  reactHooksRules,
  rawErrorMessageRestrictions,
  testFileRules,
};
