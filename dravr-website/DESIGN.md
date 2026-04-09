# Design System Strategy: The Boreal Editorial

## 1. Overview & Creative North Star

The Creative North Star for this design system is **"The Technical Naturalist."** This identity bridges the rugged, heritage-driven world of Quebec’s log drivers with a sophisticated, modern technical layer. It rejects the "web-template" aesthetic in favor of high-end editorial layouts characterized by expansive negative space, intentional asymmetry, and a tactile sense of depth.

Unlike standard digital interfaces that rely on rigid borders and shadows, this system treats the screen like a series of layered organic materials. We use high-contrast typography scales and tonal shifts to guide the eye, creating an experience that feels as much like a premium print monograph as it does a digital tool.

---

## 2. Colors & Atmospheric Depth

Our palette is anchored in the deep, atmospheric greens of the northern canopy and the warm, off-white tones of raw pulp and mist.

### The "No-Line" Rule
To maintain a high-end, seamless aesthetic, **the use of 1px solid borders for sectioning is strictly prohibited.** Division must be achieved through:
*   **Background Shifts:** Transitioning from `surface` (#F9F9F6) to `surface-container-low` (#F4F4F1).
*   **Negative Space:** Utilizing the higher end of our spacing scale (e.g., `20` or `24`) to create natural breaks.

### Surface Hierarchy & Nesting
Think of the UI as physical layers of fine paper. 
*   **Background Layer:** `surface` (#F9F9F6).
*   **Secondary Content Areas:** `surface-container-low` (#F4F4F1).
*   **Floating Elements/Cards:** `surface-container-lowest` (#FFFFFF) to provide a subtle "pop" of brightness.
*   **Interactive Overlays:** Use `surface-bright` for elements that need to command immediate attention.

### The Glass & Gradient Rule
For high-end technical components, use **Glassmorphism**. Floating navigation or detail panels should use semi-transparent `surface` colors with a `backdrop-blur(12px)`. To add "soul" to the interface, apply a subtle linear gradient to Hero sections transitioning from `primary` (#00241A) to `primary_container` (#0D3B2E) at a 145-degree angle.

---

## 3. Typography

The typography strategy relies on the tension between a technical, wide-set sans-serif and a highly legible, organic body face.

*   **The "DRAVR" Signature:** Any brand-level headers must use a modern, geometric sans-serif (Space Grotesk) with `letter-spacing: 0.15em`. This mirrors the expansive horizon of the Quebec wilderness.
*   **The Editorial Voice (Display/Headline):** Use `Space Grotesk` for all Display and Headline levels. These should be set with tight line-heights (1.1–1.2) to feel like architectural elements on the page.
*   **The Technical Voice (Body):** `Plus Jakarta Sans` is our workhorse. It provides a human, organic touch to technical data.
*   **The Functional Voice (Labels):** `Inter` is reserved for micro-copy and labels, ensuring high legibility at the smallest scales (0.6875rem).

---

## 4. Elevation & Depth

We move away from the "floating box" look toward **Tonal Layering**.

### The Layering Principle
Hierarchy is conveyed by stacking specific tokens. For a card-based layout:
1.  **Base:** `surface` (#f9f9f6)
2.  **Section:** `surface-container-low` (#f4f4f1)
3.  **Card:** `surface-container-lowest` (#ffffff)
This creates a sophisticated, "pressed" look that feels integrated rather than pasted on.

### Ambient Shadows
If a floating element (like a FAB or Popover) requires a shadow, it must mimic natural ambient light:
*   **Color:** Use a 6% opacity version of `on_surface` (#1A1C1B).
*   **Blur:** Minimum 24px.
*   **Spread:** -4px to keep it soft.

### The "Ghost Border" Fallback
If contrast is legally required for accessibility, use a **Ghost Border**: `outline_variant` (#C0C8C3) set to **15% opacity**. Never use 100% opaque borders.

---

## 5. Components

### Buttons
*   **Primary:** Filled with `primary` (#00241A), text in `on_primary` (#FFFFFF). Corner radius: `md` (0.375rem). Use a subtle 10% `surface_tint` inner glow on hover.
*   **Secondary:** Ghost style. No background, no border. Use `primary` text with an underline that expands from the center on hover.
*   **Tertiary:** Small-caps `label-md` with `letter-spacing: 0.05em`.

### Input Fields
*   **Style:** Minimalist. No enclosing box. A simple bottom stroke using `outline_variant` at 40% opacity.
*   **Focus State:** The bottom stroke becomes `primary` (#00241A) and increases to 2px.

### Cards & Lists
*   **No Dividers:** Lists should never use horizontal lines. Use `spacing-4` (1.4rem) between items.
*   **Interaction:** On hover, a list item should transition its background to `surface-container-high` (#E8E8E5) with a `md` (0.375rem) corner radius.

### Signature Component: The "Boreal Overlay"
A specialized container for technical data or image captions. It uses a semi-transparent `primary_container` (#0D3B2E at 85%) with a heavy `backdrop-blur`. It should always overlap the edge of an image or a background shift to demonstrate depth.

---

## 6. Do’s and Don’ts

### Do
*   **Do** embrace asymmetry. Balance a large image on the left with a small, right-aligned caption and significant white space.
*   **Do** use the `24` (8.5rem) spacing token for major section breaks to allow the design to "breathe."
*   **Do** use `primary_fixed_dim` (#A3D0BE) for subtle accents in technical charts or data visualizations.

### Don’t
*   **Don’t** use pure black (#000000). Always use `on_surface` (#1A1C1B) for text to maintain the organic, ink-on-paper feel.
*   **Don’t** use the `full` (9999px) roundedness scale for anything other than small tags or chips. It breaks the geometric precision of the system.
*   **Don’t** center-align long blocks of body text. Maintain a strong, left-aligned editorial rag.