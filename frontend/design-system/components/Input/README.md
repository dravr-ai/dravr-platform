# Input

Input is the editorial text field: no box, just a 1px `ghost-border` bottom rule that grows to 2px `primary` on focus.

The label is 13px, 500, sentence case, no tracking, in `on-surface-variant`. The help line is 12px `outline`. An error swaps the rule to `error` and shows a 12px `error` line. Keyboard focus adds a 2px `primary` outline at a 3px offset. `Textarea` and `Select` share this language. Never stack a boxed field beside an underlined one.

The consumer provides `label`, `error`, `helpText`, optional `leftIcon`/`rightIcon`, `size` and the native input props. On a coarse pointer the field is 16px to stop iOS zoom.
