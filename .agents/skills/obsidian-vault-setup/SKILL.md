---
name: obsidian-vault-setup
description: Use when setting up the shared dravr-vault Obsidian vault on a new machine or for
  a new team member. Guides through cloning, symlinking claude_docs, and verifying obsidian-cli.
user-invocable: true
metadata:
  version: "1.0.0"
  domain: devops
  triggers: obsidian, vault, setup, onboard, dravr-vault
  role: specialist
  scope: implementation
  output-format: report
---

# Obsidian Vault Setup

## Role

DevOps assistant for setting up the shared dravr-vault Obsidian knowledge base on a developer machine.

## When to Use

Invoke when a team member needs to:
- Clone and open the dravr-vault for the first time
- Connect their dravr-platform checkout to the vault via symlink
- Verify obsidian-cli and obsidian-git are configured correctly

## Defaults

```
PLATFORM_DIR = ~/projects/dravr-platform
VAULT_DIR    = ~/projects/dravr-vault
VAULT_REPO   = git@github.com:dravr-ai/dravr-vault.git
```

Ask the user for PLATFORM_DIR and VAULT_DIR if they differ from defaults.

## Workflow

### Step 1: Verify prerequisites

Check that the following are installed:

```bash
# Obsidian desktop app (required)
ls /Applications/Obsidian.app 2>/dev/null && echo "Obsidian: OK" || echo "Obsidian: MISSING — install from https://obsidian.md"

# obsidian-cli (optional, for scripted vault access)
which obsidian-cli 2>/dev/null && echo "obsidian-cli: OK" || echo "obsidian-cli: not installed (npm install -g obsidian-cli)"
```

### Step 2: Clone dravr-vault

```bash
# Only if not already present
if [ ! -d "$VAULT_DIR" ]; then
  git clone git@github.com:dravr-ai/dravr-vault.git "$VAULT_DIR"
fi

# Verify on main branch
cd "$VAULT_DIR" && git status
```

### Step 3: Install plugins

Downloads obsidian-git and Templater binaries and writes pre-configured settings.

```bash
cd "$VAULT_DIR"
./scripts/install-plugins.sh
```

This script:
- Downloads `main.js` + `manifest.json` for each plugin from GitHub releases
- Copies pre-configured `data.json` settings from `.obsidian/plugin-configs/`
- Writes `.obsidian/community-plugins.json` to enable both plugins

Plugin binaries are excluded from git (`.obsidian/plugins/` is in `.gitignore`).
Re-running the script is safe — it overwrites existing files idempotently.

### Step 4: Create claude_docs symlink

This links `dravr-platform/claude_docs/` to `dravr-vault/Claude Outputs/` so Claude Code
session outputs land directly in the vault.

```bash
cd "$PLATFORM_DIR"

# Safety check: do NOT overwrite a real directory
if [ -d claude_docs ] && [ ! -L claude_docs ]; then
  echo "ERROR: claude_docs/ exists as a real directory. Back it up before proceeding."
  exit 1
fi

ln -sf "$VAULT_DIR/Claude Outputs" claude_docs
ls -la claude_docs   # verify symlink resolves
```

### Step 5: Open vault in Obsidian

Instruct the user to:
1. Open Obsidian
2. Click **Open folder as vault**
3. Navigate to `$VAULT_DIR` and click **Open**
4. When prompted "Trust and enable plugins?", click **Trust author and enable plugins**

Both obsidian-git and Templater will be active immediately — no manual browsing required.

### Step 6: Verify sync flow

After obsidian-git is configured:

```bash
# Create a test file in Claude Outputs
echo "# Test" > "$PLATFORM_DIR/claude_docs/test.md"

# Verify it appears in the vault
ls "$VAULT_DIR/Claude Outputs/test.md"

# Clean up
rm "$PLATFORM_DIR/claude_docs/test.md"
```

## Constraints

- NEVER overwrite an existing `claude_docs/` directory that contains real files
- ALWAYS verify dravr-vault is on `main` branch before linking
- NEVER install obsidian-cli using yarn or pnpm — use `npm install -g obsidian-cli` (global tool, not a project dependency)
- If PLATFORM_DIR or VAULT_DIR differ from defaults, ask for them before running any commands

## Success Criteria

- `ls -la $PLATFORM_DIR/claude_docs` shows a symlink pointing to `$VAULT_DIR/Claude Outputs`
- Obsidian opens the vault and shows all folders (Architecture, APIs, Methodology, Development)
- obsidian-git plugin is active and auto-push is enabled
