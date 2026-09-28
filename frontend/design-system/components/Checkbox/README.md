# Checkbox

Checkbox and Radio are the choice controls: a 16px box (radius 4) or circle whose edge is `outline`, which clears 3:1, not a hairline.

When checked, the box fills with `primary` and shows an `on-primary` tick, and the radio shows a `primary` dot. The label is 13px `on-surface`, with an optional 12px `outline` description under it. On a tinted hue, pass that hue's `on-*-container` ink as `labelClassName`. The consumer provides `label`, `description`, `error` and the native input props.
