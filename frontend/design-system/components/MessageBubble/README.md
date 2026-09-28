# MessageBubble

MessageBubble draws one turn of the thread. The agent speaks as prose on the canvas, and the athlete speaks in the one bubble.

- **Agent** (`side="assistant"`): a 24px initials avatar in the gutter, then the name (13px, 600) and the time (12px `outline`) on one line, then the words at 15/23 up to 620px. No fill, no border, no radius.
- **Athlete** (`.chat-bubble-user`): right-aligned on `primary-container` with `on-primary-container` text, radius 14 with a 4px bottom-right tail, and at most 65% wide on desktop (85% below `lg`). A filled `primary` would read as a CTA.
- The typing indicator is three `outline` dots breathing over 1.4s, where the agent's words will be. Actions (copy, rate, regenerate) show only on hover, focus or a coarse pointer.

The consumer provides `side`, `children`, and for an agent run `authorLabel`, `avatar`, `groupStart`, a 24-hour `timestamp`, `actions` and `footer` (the verdict chip).
