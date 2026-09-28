# Design System artifact

This folder produces **Dravr Boreal**, the Claude Design System artifact at
<https://claude.ai/artifact/NGizBhmoqTqmx2h2fGBXMY>. The artifact is a browsable copy of this
system: tokens with usage notes, the brand book, the Boreal Ripple marks, and live previews that
run the web app's own `ui/` components. It is a derived view. `DESIGN.md` and
`packages/shared-constants/src/design-system.ts` remain the source of truth.

## Regenerate

```bash
cd frontend
bun run design-system
```

This writes the artifact's files to `design-system/dist/project/` (gitignored):

| File | Built from |
| --- | --- |
| `tokens.json` | `design-system.ts` (colours, hairlines, spacing, radii, shadow), `tailwind.config.cjs` (web type scale, font stacks), `frontend-mobile/src/constants/typeScale.js` (phone scale), with the prose from `token-notes.json` |
| `components/bundle.js` | `bundle-entry.ts`: the web primitives as one IIFE assigning `window.Boreal`, reading React from the page's globals |
| `components/bundle.css` | `src/index.css` compiled by the app's Tailwind config, purged to the bundled components and the previews |
| `README.md` | `brand-book.md` |
| `components/*/README.md`, `preview.html` | this folder's `components/` |
| `design-system.json` | `artifact.json`: the title, namespace and the uploaded logo records |

The generator fails rather than writes a partial system. It fails when a colour, spacing, radius or
shadow value has no note in `token-notes.json`, or a note names no value. It also fails when a card
folder is not imported by `bundle-entry.ts`, when `DravrLogo`'s image paths can no longer be pointed
at the uploaded marks, when the Tailwind run produces a sheet without the component classes, or
when the bundle contains `</script`.
`src/__tests__/DesignSystemExport.test.ts` runs it end to end and mounts the bundled `Button`
with its card's preview script.

## Publish

Publishing needs the Artifact tool, so it goes through Claude Code. Ask it to republish
`frontend/design-system/dist/project/` to the artifact URL above, following the Design System
type's revising steps: read the live index first, then send only the changed files.

## Change something

- **A token value**: change it in `design-system.ts` (or the Tailwind config), then regenerate. Nothing here restates it.
- **A new token**: add its usage line to `token-notes.json`, or the generator refuses to run.
- **A new card**: import the component in `bundle-entry.ts` and add `components/<Name>/README.md` and `preview.html`. The preview's first line is `<!-- @dsCard group="…" height=N -->`, and its script renders `window.Boreal.<Name>` with `React.createElement`.
- **The logos**: they are uploads in the artifact's asset store, recorded by id in `artifact.json`. Re-upload through the Artifact tool and update the records.
