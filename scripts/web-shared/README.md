# dravr-web-shared

**A generated mirror. Do not edit or push here.**

TypeScript packages shared by the Dravr web app (dravr-platform `frontend/`) and the
React Native app (dravr-pitoune). Their one source is dravr-platform `packages/`: three of
them are generated from the Rust server there (`scene-types`, `shared-types`,
`shared-constants/src/*.generated.ts`), and the web app that consumes all of them lives there.

Every commit here is written by dravr-platform's `scripts/ci/web-shared-mirror.sh`, run by
the `Satellite: dravr-web-shared mirror` workflow on each main push that touches `packages/`.
It carries a `Synced-From: dravr-platform@<sha>` trailer naming the commit it mirrors; the
same run moves dravr-pitoune's `packages/` submodule to it. A commit without that trailer
stops the next sync until its change is ported into dravr-platform.

To change a shared package: edit `packages/` in dravr-platform and land it there. This file
and `package.json` come from dravr-platform `scripts/web-shared/`.
