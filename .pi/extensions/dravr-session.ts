// ABOUTME: Pi session extension porting this repo's Claude Code SessionStart and
// ABOUTME: PostToolUse hooks — repo bootstrap, vault memory sync, environment checks.

import { existsSync } from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const BOOTSTRAP_SCRIPT = ".build/ci/bootstrap-repo.sh";

/** Scripts whose output belongs in the model's context at session start. */
const STARTUP_CHECKS: { label: string; script: string; args: string[] }[] = [
	{ label: "vault-memory", script: "../dravr-vault/scripts/sync-claude-memory.sh", args: ["--quiet"] },
	{ label: "gh-cli", script: "./scripts/setup/check-gh-cli.sh", args: [] },
	{ label: "mcp-tokens", script: "./scripts/setup/check-mcp-tokens.sh", args: [] },
];

export default function (pi: ExtensionAPI) {
	pi.on("session_start", async (_event, _ctx) => {
		const lines: string[] = [];

		// The .build submodule carries the canonical hooks and validation scripts. A fresh
		// clone or worktree has the directory but no checkout, so init before running it.
		if (!existsSync(BOOTSTRAP_SCRIPT)) {
			await pi.exec("git", ["submodule", "update", "--init", "--recursive", "-q"], { timeout: 120000 });
		}
		if (existsSync(BOOTSTRAP_SCRIPT)) {
			const bootstrap = await pi.exec("bash", [BOOTSTRAP_SCRIPT], { timeout: 120000 });
			const output = `${bootstrap.stdout}${bootstrap.stderr}`.trim();
			if (output) lines.push(`[bootstrap] ${output}`);
		}

		for (const check of STARTUP_CHECKS) {
			if (!existsSync(check.script)) continue;
			const result = await pi.exec("bash", [check.script, ...check.args], { timeout: 60000 });
			const output = `${result.stdout}${result.stderr}`.trim();
			if (output) lines.push(`[${check.label}] ${output}`);
		}

		if (lines.length === 0) return;

		pi.sendMessage(
			{
				customType: "dravr-startup",
				content: lines.join("\n"),
				display: true,
			},
			{ deliverAs: "nextTurn" },
		);
	});

	// Claude Code echoed this nudge on every matching edit. Once per session is enough to
	// land it in context, and avoids repeating the same line through a long refactor.
	let designReviewNudged = false;

	pi.on("tool_call", async (event, _ctx) => {
		if (designReviewNudged) return;
		if (event.toolName !== "write" && event.toolName !== "edit") return;
		if (!JSON.stringify(event.input ?? {}).includes("frontend/src/components")) return;

		designReviewNudged = true;
		pi.sendMessage(
			{
				customType: "dravr-design-review",
				content: "Frontend component modified — consider running /skill:design-review.",
				display: true,
			},
			{ deliverAs: "nextTurn" },
		);
	});
}
