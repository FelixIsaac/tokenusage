# Project Roadmap & Vision

This document outlines the strategic roadmap for **tokenusage** as a real-time developer telemetry, rate-limit protection, and multi-agent control center for AI coding assistants.

---

## 🎯 Core Philosophy & Direction

1. **Zero-Cloud Privacy & Speed:** 100% local log and database parsing in under **0.08 seconds**. No telemetry or logs ever leave the developer's machine.
2. **Rate-Limit & Capacity Shield:** Keep developers from hitting unexpected quota walls or rate limits mid-refactor.
3. **Unified Multi-Agent Hub:** Single local control plane across **Claude Code, OpenAI Codex, Antigravity/AGY, Gemini CLI, OpenCode, Grok Build, DeepSeek, OpenRouter, Kimi, and Anthropic API**.

---

## 🗺️ Upcoming Milestones

### Phase 1: Rebranding & Scoped Registries (Q3 2026)
- [x] **GitHub Fork Detachment:** Detached `FelixIsaac/tokenusage` into an independent root repository network.
- [ ] **Rebranding Evaluation:** Transition brand towards `codetop` or `aipulse` (or publish as `@felixisaac/tokenusage` on NPM and `tokenusage-ext` on Crates.io) to avoid collisions with inactive upstream domains.
- [ ] **Binary Aliases:** Maintain `tu` binary compatibility alongside any new brand command.

### Phase 2: Desktop Notifications & Threshold Alerts (Q3 2026)
- [ ] **Native OS Notifications:** Trigger desktop notifications (macOS Notification Center, Windows Toast, Linux `notify-send`) when quota falls below `--warn-threshold` (e.g. 15% remaining).
- [ ] **Statusline Hooks:** Expand JSON & widget output for statusline hooks across `starship`, `oh-my-zsh`, and `ccstatusline`.

### Phase 3: Expanded AI Agent Support (Q4 2026)
- [ ] **Cursor & Windsurf Log Parsers:** Add parser engines for Cursor and Windsurf local session databases/transcripts.
- [ ] **Aider & Continue.dev Support:** Parse local session history from Aider (`.aider.chat.history.md`) and Continue.dev logs.
- [ ] **Enhanced GUI Dashboard (`tu gui`):** Add interactive filtering, model cost comparison charts, and export tools to the `iced` desktop app.

---

## 🤝 Community & Feedback

If you have feature requests, new provider requests, or feedback, please open an issue on GitHub:
👉 [https://github.com/FelixIsaac/tokenusage/issues](https://github.com/FelixIsaac/tokenusage/issues)
