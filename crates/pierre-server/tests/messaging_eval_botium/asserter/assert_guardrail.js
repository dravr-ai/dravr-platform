// ABOUTME: JS Botium asserter mirroring the Rust assert_guardrail helper
// ABOUTME: Fuzzy substring match after whitespace normalization; same contract as helpers::messaging_eval::assert_guardrail

// Custom Botium asserter: verifies the bot reply contains an expected
// guardrail substring after case-insensitive whitespace normalization.
// Same contract as the Rust `assert_guardrail` helper in
// `tests/helpers/messaging_eval.rs` so scenarios that assert on
// guardrail copy here behave the same way as the Rust integration
// tests do.
//
// Not auto-registered — referenced by BotiumScript scenarios via the
// asserter/ directory once Phase 2 activation wires the connector up
// to a real Slack workspace.

function normalize(text) {
  return String(text)
    .split(/\s+/)
    .filter((t) => t.length > 0)
    .map((t) => t.toLowerCase())
    .join(' ')
}

class AssertGuardrail {
  constructor(_context, _caps = {}) {
    this.name = 'ASSERT_GUARDRAIL'
  }

  // args[0] is the expected substring supplied by the scenario
  // (e.g. CUSTOM ASSERT_GUARDRAIL outside what I can help with).
  assertConvoStep({ convoStep, args = [], botMsg }) {
    const expected = (args[0] || '').toString()
    if (!expected) {
      return Promise.reject(
        new Error(
          `[${convoStep.stepTag}] ASSERT_GUARDRAIL needs one argument: the expected refusal substring`,
        ),
      )
    }
    const reply = normalize(botMsg && botMsg.messageText ? botMsg.messageText : '')
    const needle = normalize(expected)
    if (reply.includes(needle)) {
      return Promise.resolve()
    }
    return Promise.reject(
      new Error(
        `[${convoStep.stepTag}] guardrail string not found\n  expected (normalized): "${needle}"\n  reply    (normalized): "${reply}"`,
      ),
    )
  }
}

module.exports = AssertGuardrail
