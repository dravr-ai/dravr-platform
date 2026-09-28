# DravrLogo

DravrLogo renders the Boreal Ripple mark as a theme-aware image: forest (`mark-ink-*`) on light and mint (`mark-mint-*`) on dark, picking the smallest file at least twice the rendered size.

The consumer provides `size` in px (40 by default, the rail size) and an optional `className`. The mark is `aria-hidden`, because every placement sits beside the DRAVR wordmark or inside chrome that already names the app.

**The lockup** is the mark at 28–32px beside the wordmark, set in HTML: `font-display text-xl font-semibold tracking-brand text-primary` (Schibsted Grotesk 600, 0.15em, `primary` ink). Use it in the admin sidebar, on auth pages, in onboarding, and in the phone's Home and Chat headers. The web rail shows the mark alone at 40px, and the login hero shows it at 220px. Never bake the wordmark into the mark, and never put the mark on a plate, badge or gradient.

In this system's bundle, the mark's image paths point at the uploaded Logos files instead of the app's `/brand/` folder.
