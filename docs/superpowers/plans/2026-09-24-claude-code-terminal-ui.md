# Claude Code Terminal UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. If you run in Codex CLI without those skills, execute the tasks in order, one commit per task, and stop at every "Checkpoint" to show the user the result.

**Goal:** Make the davinci interactive shell look and respond like Claude Code 2.1.281: the same composer, completion list, footer, transcript rows, working line, permission panel and picker panels, using the captured reference frames as the ground truth.

**Architecture:** All colors come from one new palette, `Theme::cc()`, whose values are the colors measured on the live Claude Code screens in `docs/ui/claude-code-reference/`. Views keep their current files and functions; each task rewrites one view to the reference layout and pins it with ratatui buffer tests (text, column and color per span). Behavior changes are limited to the composer's Enter key on slash lists, a `?` shortcuts panel, and shell-mode rendering.

**Tech Stack:** Rust 1.83, ratatui, crossterm, pulldown-cmark (already in `davinci-tui`). Capture tools: Python 3 with `pywinpty` and `pyte` (maintainer only, never in tests).

## Global Constraints

- The reference frames in `docs/ui/claude-code-reference/` are the authority. When this plan and a frame disagree, the frame wins; note the difference in the PR.
- Read a frame with `python scripts/ui/show_styles.py docs/ui/claude-code-reference/<set> <frame> [substring]`. Each run prints `[fg,bg=…,bold,dim]'text'`. Leading blank cells are not printed, so count columns in the matching `.txt` file.
- Do not copy Claude Code source, names, logo or mascot. The product name stays "DaVinci", the logo stays DaVinci's block letters, and no string says "Claude".
- Colors appear only in `crates/davinci-tui/src/davinci/theme.rs` (module rule in its doc comment). Views read `model.theme.cc()` or the existing theme tokens.
- `NO_COLOR` keeps working: every state still has a glyph or a text label, and no view draws a background color under `NO_COLOR`.
- Narrow terminals keep working: no row may exceed `model.width`; keep every existing "never overruns the window" test green.
- Tests are offline and fixture-only. Never start `claude`, `davinci` or a network call from a test.
- Toolchain Rust 1.83.0. Add no new crate dependency.
- Windows shell is PowerShell; prefix cargo with `rtk`: `rtk cargo test -p davinci-tui --offline <filter>`.
- Never commit on `main`. Branch per part: `ui-cc-a-input`, `ui-cc-b-transcript`, `ui-cc-c-panels`, each from the latest `origin/main`, each opened as its own PR for human merge. If the Windows pre-commit hook fails with `execvpe(/bin/bash) failed`, commit with `--no-verify` (see CLAUDE.md).
- Commit messages and docs: plain English, no em dashes.
- The `vox`, `vox-classic` and `light` themes keep working; `vox` only changes the existing ramp and uses the same `cc()` palette as dark.

## Reference Captures

Captured on 2026-09-24 from Claude Code 2.1.281 at 120 x 40 in a Windows pseudo console, truecolor, hooks disabled. Local paths are replaced by `~\project`, the plan tier by `<plan>`, and the logo cells by spaces.

| Set | Frames | What they show |
|---|---|---|
| `ui` | `01-welcome` … `26-enter-on-comp` | welcome, `?` shortcuts, `/` list, `/comp`, `/mo`, `@` list, `!` shell mode, typed and multi-line composer, the five permission modes, `/model`, `/config`, `/permissions`, `/resume`, `/help`, double Esc, Enter on `/comp` |
| `turn` | `30-turn-*`, `31-permission-1`, `34-final`, `35-ctrl-o` | a live turn: working line frames, grouped read and shell calls, edit approval, diff, markdown reply, completion line, verbose transcript |
| `turn2` | `31-permission-2`, `41-permission-3/4`, `40-turn-005`, `34-final`, `44-final` | create-file approval, shell approval, working line with tip row, replace diff with removal, write result |
| `shell` | `50`–`55` | `!` command echo and output, `(No output)`, Tab on `/`, Tab on `/mod`, Enter with `@` |

To capture again (maintainer only; spends Claude usage for the `turn` phase):

```powershell
pip install pywinpty pyte
python scripts/ui/capture_tui.py --phase ui --out $env:TEMP\cc-ui --sandbox <small git repo>
python scripts/ui/capture_tui.py --phase ui --bin (Get-Command davinci).Source --out $env:TEMP\dv-ui --sandbox <same repo>
```

The davinci binary writes no color codes inside this pseudo console, so its captures compare text only. Colors are checked by the ratatui buffer tests in this plan.

## The Contract

Every value below was read from the frames. Column numbers start at 0 on a 120-column screen.

### Palette (`Theme::cc()`, dark truecolor)

| Token | Hex | Where it is seen |
|---|---|---|
| `claude` | `d77757` | logo, working glyph and verb, `Opus 5.5` in the model picker, effort dot in the picker |
| `claude_shimmer` | `eb9f7f` | 3-letter highlight moving across the working verb |
| `permission` | `b1b9f9` | selected completion row, inline code, `/model` inside text, panel rules and titles, selected option |
| `inactive` | `999999` | footer, effort line, `⎿` elbow, unselected completion rows, counts, descriptions |
| `inactive_shimmer` | `b9b9b9` | shimmer on `thinking with high effort` |
| `subtle` | `505050` | `❯ ` in an echoed user message, dashed `╌` rules, `←/→ to adjust` |
| `prompt_border` | `888888` | the two composer rules |
| `text` | `ffffff` | echoed user text, assistant bullet `●` |
| `user_bg` | `373737` | echoed user message background |
| `bash` | `fd5db1` | shell-mode rules, `!` prompt, `! for shell mode`, `! ` in an echoed shell command |
| `bash_bg` | `413c41` | echoed shell command background |
| `success` | `4eba65` | finished tool bullet, current model `✔` row |
| `error` | `ff6b80` | failed tool bullet (not in the frames; Claude Code's error red) |
| `auto_mode` | `ffc107` | `⏵⏵ auto mode on` |
| `plan_mode` | `48968c` | `⏸ plan mode on` |
| `accept_edits` | `af87ff` | `⏵⏵ accept edits on` |
| `diff_text` | `f8f8f2` | diff text and line numbers (numbers also dim) |
| `diff_add` / `diff_add_bg` | `50c850` / `022800` | added row sign and number / row background |
| `diff_del` / `diff_del_bg` | `dc5a5a` / `3d0100` | removed row sign and number / row background |
| `code_keyword` / `code_string` / `code_macro` | ANSI blue / red / cyan | fenced code in replies (`let`, `"hello"`, `println!`) |

Plain body text and markdown prose use the terminal default foreground (`Color::Reset`), as the frames do.

### Composer (`ui/01-welcome`, `ui/12-typed`, `ui/13-multiline`, `ui/09-bang`)

```
row 35:                                                                                                       ● high · /effort   <- all inactive, right aligned, 2 cells from the edge
row 36: ────────────────────────────────────────────────────────────────  <- prompt_border, full width
row 37: ❯ Try "fix lint errors"                                           <- "❯" + U+00A0, then the placeholder in default fg + DIM
row 38: ────────────────────────────────────────────────────────────────  <- prompt_border
row 39:   ⏸ manual mode on · ? for shortcuts                              <- footer, see below
```

- Typed text: default fg. Continuation rows of a multi-line draft start with two spaces.
- While a turn runs the prompt `❯` is `inactive` and the composer stays usable (`turn/30-turn-003`).
- Shell mode, when the draft starts with `!`: both rules, the prompt glyph and the footer are `bash`; the prompt glyph is `!` + U+00A0 and the leading `!` is not repeated in the text (`ui/10-bang-typed`: `! git status`). The footer is only `! for shell mode`.
- A slash command at the start of the draft is drawn in `permission` (`shell/53-tab-on-slash`). After Tab completes `/model`, its argument hint follows in `inactive` (`shell/54-tab-mod`: `/model [model]`).

### Completion list (`ui/03`, `04`, `05`, `06`, `07`)

- Drawn directly above the effort line and top rule, no blank row, no border, no footer.
- At most 5 rows. Each item takes 1 or 2 rows; only whole items are shown; the selected item is always visible.
- Command row: two spaces, `/name` padded to the name column, then the description. The name column is `max(20, width * 2 / 5)` cells wide, so descriptions start at column 50 on 120 columns. A description wraps to at most 2 rows in `width - 2 - name_column - 2` cells; the second row ends in `…` when text is cut. Continuation rows are indented to the description column.
- File row (`@`): two spaces, `+ `, the path with the platform separator, a trailing separator on directories (`+ .git\`).
- The selected item (all its rows, name and description) is `permission`. Every other item is `inactive`.

### Keys in the composer (`ui/26-enter-on-comp`, `shell/53`, `shell/54`)

- Enter with a slash-command list open runs the highlighted command (`/comp` + Enter ran `/compact`).
- Enter with a slash-argument list open (`/thinking h`) accepts the argument and runs the command.
- Tab completes without running.
- Enter with an `@` file list open inserts the highlighted path and does not send.
- `?` in an empty composer shows the shortcuts panel in place of the footer; any other key hides it and is handled normally.

### Footer (`ui/14`–`18`, `turn/30-turn-001`)

| davinci mode | Idle footer (colors) | While running |
|---|---|---|
| `ask` (Manual) | `  ⏸ manual mode on · ? for shortcuts` (all `inactive`) | `  ⏸ manual mode on · esc to interrupt` |
| `edits` | `  ⏵⏵ accept edits on` (`accept_edits`) + ` (shift+tab to cycle)` (`inactive`) | idle text + ` · esc to interrupt` |
| `read-only` | `  ⏸ plan mode on` (`plan_mode`) + ` (shift+tab to cycle)` | same rule |
| `auto` | `  ⏵⏵ auto mode on` (`auto_mode`) + ` (shift+tab to cycle)` | same rule |
| `always-approve` | `  ⏵⏵ always approve on · no prompts` (`error`) + ` (shift+tab to cycle)` | same rule |

- `shift+tab` is the first key bound to `app.permissions.cycle`.
- When background agents exist, append ` · ← N agent` / ` · ← N agents` (`inactive`). With none, append nothing.
- `ctrl+c` armed: the whole footer is `  Press Ctrl-C again to exit` (`inactive`).
- Shortcuts panel (`ui/02-shortcuts`): three columns starting at columns 2, 26 and 61, all `inactive`; below 100 columns, one column.

### Transcript (`turn/34-final`, `turn2/44-final`, `turn/35-ctrl-o`, `shell/52-bang-fail`)

- A blank row separates every block.
- User message: `❯ ` (`subtle`) + text (`text`), background `user_bg` from column 0 to one cell past the text on every row; inline code inside it is `permission` without backticks. Wrapped rows start with two spaces (also on `user_bg`).
- Echoed shell command: `! ` (`bash`) + command (`text`) on `bash_bg`, then `  ⎿  ` (`inactive`) + first output row (default fg); later output rows are indented 5 spaces; empty output is `  ⎿  (No output)` all `inactive`.
- Assistant prose: `● ` (`text`) on the first row, two spaces on the rest, markdown in default fg.
- Markdown: headings are bold with no `#` and no rule; `- ` bullets and `1.`/`a.` labels are kept as written; inline code and file names in backticks are `permission` without backticks; strikethrough kept; fenced code has no fence, no language row and no left rule, sits at the prose column, and uses `code_keyword`, `code_string`, `code_macro`; everything else default fg.
- Tool call: `● ` then the tool name bold (default fg) then `(argument)` default fg. The bullet is `success` when finished, `inactive` and blinking while running, `error` when failed.
- Tool result: `  ⎿  ` (`inactive`, the elbow followed by a space and U+00A0) then the summary in default fg with every number bold: `Read 4 lines`, `Added 2 lines, removed 1 line`, `Wrote 2 lines to notes.txt`.
- Consecutive read, search, list and shell calls fold into one row: finished, `  Read 1 file, ran 1 shell command` (`inactive`, numbers bold, no bullet); running, `● Reading 1 file, running 1 shell command…` (bullet `inactive`, text default, numbers bold) plus `  ⎿  $ echo hello` (`inactive`) naming the newest call. `ctrl+o` (verbose) shows every call separately.
- Diff rows under a result: 5 spaces, then ` N ` (line number right-aligned to the widest number in the block), then the sign (` `, `+`, `-`), then the text. Context: number `diff_text` + DIM, text `diff_text`. Added: number and `+` `diff_add` on `diff_add_bg`, text `diff_text` on `diff_add_bg`, background to the right edge. Removed: same with `diff_del`/`diff_del_bg`.
- Reasoning text is not shown in the normal view. The working line says `thinking with high effort` and then `thought for 1s`.
- Turn end: `✻ Worked for 7s` (glyph and text `inactive`).

### Working line (`turn/30-turn-000` … `30-turn-008`, `turn2/40-turn-005`)

```
✢ Levitating… (2s · ↓ 138 tokens · thinking with high effort)
```

- Row directly above the effort line or composer, starting at column 0.
- Glyph frames `·`, `✢`, `*`, `✶`, `✻`, `✽`, then back down (`✻`, `✶`, `*`, `✢`), one step per tick; glyph `claude`.
- One verb for the whole turn, `claude`, with a 3-letter window in `claude_shimmer` moving one letter per tick; `… ` `claude`.
- `(` … `)` and the facts are `inactive`: seconds, `↓ N tokens` once tokens arrive, then `thinking with <effort> effort` (alternating `inactive_shimmer` and `inactive` per tick) while reasoning streams, or `thought for Ns` after it. `esc to interrupt` moves to the footer.
- `Interrupting…` replaces the verb after Esc.

### Permission panel (`turn/31-permission-1`, `turn2/31-permission-2`, `turn2/41-permission-3`)

Replaces the composer while open:

```
────────────────────────────────────────────  <- permission, full width
 Edit file                                    <- bold permission; "Create file", "Bash command", "PowerShell command", "<Tool> tool"
 README.md                                    <- inactive (file tools)
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌  <- subtle
 1  # Sandbox                                 <- diff rows, gutter at column 0
 4 +Edited.
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Do you want to make this edit to README.md?  <- default fg, file name bold
 ❯ 1. Yes                                     <- ❯ and label permission, "1. " inactive
   2. Yes, and don't ask again for: cargo *   <- "2. " inactive, label default
   3. No
                                              <- blank
 Esc to cancel                                <- inactive
```

Shell calls show a blank row, `   <command>` (3 spaces, default fg), a blank row and ` Do you want to proceed?`. Write calls ask ` Do you want to create notes.txt?`.

### Picker panels (`ui/19-model-picker`, `ui/22-permissions`, `ui/24-help`)

- Top rule `▔` in `permission` across the width, with ` ● <effort> · /effort ▔` at its right in `inactive` (davinci already draws this through `chrome::effort_rule`; only the rule color changes).
- Title at column 3, bold `permission`. Description rows at column 3, `inactive`.
- Rows: selected `   ❯ N. label` (`❯` `permission`, `N. ` `inactive`, label `permission`); others `     N. label` (`N. ` `inactive`, label default). The current value adds ` ✔` and both label and `✔` are `success`.
- Model picker effort row: `   ● High effort ←/→ to adjust` (`●` `claude`, text `inactive`, hint `subtle`).
- Footer hint at column 3, `inactive`.

## Intentional Differences

List these in `docs/ui/design.md` and in each PR:

1. DaVinci name, version and block-letter logo instead of Claude Code's name and mascot; no plan tier in the banner.
2. No `· done 2:24 AM` clock on the completion line (it needs local time zones and a new dependency).
3. `!` commands do not start a model reply afterwards (pi behavior; saves usage). Claude Code 2.1.281 answers them.
4. Failed calls are never folded into the grouped row; they keep their own `●` line and first four output rows.
5. Always Approve keeps the words `no prompts` for safety.
6. The voice `[mic …]` label on the top rule stays when voice is enabled.
7. No tip row under the working line and no `/btw`.
8. Light theme values are Claude Code's light theme as documented, not captured (see Task A1).

## File Structure

| File | Change |
|---|---|
| `crates/davinci-tui/src/davinci/theme.rs` | add `Cc` palette and `Theme::cc()` |
| `crates/davinci-tui/src/davinci/app.rs` | Enter on slash lists; `?` panel toggle; footer rows in `compose` |
| `crates/davinci-tui/src/davinci/model.rs` | `shortcuts_open`, `placeholder` fields; `Working.turn_start`, `Working.verb_seed`; `Entry::Shell`, `Entry::Done`; `Hunk.line` |
| `crates/davinci-tui/src/davinci/views/chrome.rs` | completion list, composer, effort line, footer, shortcuts panel |
| `crates/davinci-tui/src/davinci/views/opera.rs` | working line |
| `crates/davinci-tui/src/davinci/views/transcript.rs` | user echo, shell echo, tool rows, grouping, diffs, done line, hidden reasoning |
| `crates/davinci-tui/src/davinci/ui.rs` | `tool_line`, `section_row`, `hint_row` colors |
| `crates/davinci-tui/src/davinci/views/markdown.rs` | Claude Code markdown rules |
| `crates/davinci-tui/src/davinci/views/highlight.rs` | none (Token kinds reused) |
| `crates/davinci-tui/src/davinci/views/startup.rs` | banner |
| `crates/davinci-tui/src/davinci/views/ask.rs` | permission panel |
| `crates/davinci-tui/src/davinci/views/cogitator.rs` | model picker rows and effort row |
| `crates/davinci-coding-agent/src/davinci_interactive.rs` | shell echo entries, turn verb seed and done line, `hunks_from_diff` line numbers, approval preview |
| `docs/ui/design.md`, `CLAUDE.md` | document the new contract |

## Tests That Will Change

Many existing tests pin the old look. When one fails, apply this table: if it asserts a value in the left column, change the expectation to the right column. If it asserts behavior (row counts, width bounds, `NO_COLOR` glyphs, Unicode caret safety, focus routing), keep it and fix the code.

| Old | New |
|---|---|
| working glyphs `◜◝◞◟`, `(esc to interrupt · 12s …)` | frames `·✢*✶✻✽`, `(12s · ↓ 423 tokens · thinking with high effort)` |
| verb changes every 3 s | one verb per turn |
| completion rows with selection bar, `… N above/below`, `N commands · ↑↓ move · tab take · esc close` | Claude Code rows, no counters, no footer |
| inline code in `theme.secondary` | `cc.permission` |
| bullets `·`, code behind `│ `, language row, rule under `#`/`##` | `- `, no rule, no language row, no heading rule |
| hunks `│ + text` with no numbers | numbered diff rows |
| `⟐ reasoned 3s · …` in the normal view | nothing (verbose view keeps it) |
| footer `· ? for shortcuts · ← for agents` for every mode | table in "Footer" |
| `DaVinci` as a paper label | bold text |

---

# Part A: Input and composer (branch `ui-cc-a-input`)

### Task A1: Claude Code palette

**Files:**
- Modify: `crates/davinci-tui/src/davinci/theme.rs`
- Test: same file, `mod tests`

**Interfaces:**
- Produces: `pub struct Cc { … }` with the fields below, and `impl Theme { pub fn cc(&self) -> Cc }`. Every later task reads colors as `let cc = model.theme.cc();`.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `theme.rs`)

```rust
    #[test]
    fn cc_palette_matches_the_claude_code_reference() {
        let cc = Theme::da_vinci(ColorDepth::TrueColor, false).cc();
        assert_eq!(cc.claude, Color::Rgb(0xD7, 0x77, 0x57));
        assert_eq!(cc.claude_shimmer, Color::Rgb(0xEB, 0x9F, 0x7F));
        assert_eq!(cc.permission, Color::Rgb(0xB1, 0xB9, 0xF9));
        assert_eq!(cc.inactive, Color::Rgb(0x99, 0x99, 0x99));
        assert_eq!(cc.subtle, Color::Rgb(0x50, 0x50, 0x50));
        assert_eq!(cc.prompt_border, Color::Rgb(0x88, 0x88, 0x88));
        assert_eq!(cc.user_bg, Color::Rgb(0x37, 0x37, 0x37));
        assert_eq!(cc.bash, Color::Rgb(0xFD, 0x5D, 0xB1));
        assert_eq!(cc.bash_bg, Color::Rgb(0x41, 0x3C, 0x41));
        assert_eq!(cc.success, Color::Rgb(0x4E, 0xBA, 0x65));
        assert_eq!(cc.auto_mode, Color::Rgb(0xFF, 0xC1, 0x07));
        assert_eq!(cc.plan_mode, Color::Rgb(0x48, 0x96, 0x8C));
        assert_eq!(cc.accept_edits, Color::Rgb(0xAF, 0x87, 0xFF));
        assert_eq!(cc.diff_add_bg, Color::Rgb(0x02, 0x28, 0x00));
        assert_eq!(cc.diff_del_bg, Color::Rgb(0x3D, 0x01, 0x00));
        // vox changes only the editorial ramp, never this palette.
        let vox = Theme::da_vinci(ColorDepth::TrueColor, false).with_name("vox");
        assert_eq!(vox.cc(), cc);
    }

    #[test]
    fn cc_palette_has_no_color_under_no_color() {
        for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256, ColorDepth::Basic] {
            let cc = Theme::da_vinci(depth, true).cc();
            for color in [cc.claude, cc.permission, cc.inactive, cc.user_bg, cc.bash_bg, cc.diff_add_bg] {
                assert_eq!(color, Color::Reset, "{depth:?}");
            }
        }
    }

    #[test]
    fn cc_palette_keeps_the_terminal_encoding() {
        let cc = Theme::da_vinci(ColorDepth::Ansi256, false).cc();
        assert!(matches!(cc.claude, Color::Indexed(_)));
        assert!(matches!(cc.user_bg, Color::Indexed(_)));
        let cc = Theme::da_vinci(ColorDepth::Basic, false).cc();
        assert!(!matches!(cc.claude, Color::Rgb(..) | Color::Indexed(_)));
    }

    #[test]
    fn cc_palette_drops_to_inactive_behind_a_modal() {
        let dim = Theme::da_vinci(ColorDepth::TrueColor, false).dim().cc();
        assert_eq!(dim.permission, dim.inactive);
        assert_eq!(dim.claude, dim.inactive);
        assert_eq!(dim.user_bg, Color::Reset);
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `rtk cargo test -p davinci-tui --offline cc_palette`
Expected: FAIL, `no method named cc`.

- [ ] **Step 3: Implement** (add after the `BASIC_GREY` constant, before `impl Theme`)

```rust
/// Claude Code conversation colors, measured on Claude Code 2.1.281 frames
/// (docs/ui/claude-code-reference). Views take these through `Theme::cc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cc {
    pub claude: Color,
    pub claude_shimmer: Color,
    pub permission: Color,
    pub inactive: Color,
    pub inactive_shimmer: Color,
    pub subtle: Color,
    pub prompt_border: Color,
    pub text: Color,
    pub user_bg: Color,
    pub bash: Color,
    pub bash_bg: Color,
    pub success: Color,
    pub error: Color,
    pub auto_mode: Color,
    pub plan_mode: Color,
    pub accept_edits: Color,
    pub diff_text: Color,
    pub diff_add: Color,
    pub diff_add_bg: Color,
    pub diff_del: Color,
    pub diff_del_bg: Color,
    pub code_keyword: Color,
    pub code_string: Color,
    pub code_macro: Color,
}

const CC_DARK: Cc = Cc {
    claude: rgb(0xD77757),
    claude_shimmer: rgb(0xEB9F7F),
    permission: rgb(0xB1B9F9),
    inactive: rgb(0x999999),
    inactive_shimmer: rgb(0xB9B9B9),
    subtle: rgb(0x505050),
    prompt_border: rgb(0x888888),
    text: rgb(0xFFFFFF),
    user_bg: rgb(0x373737),
    bash: rgb(0xFD5DB1),
    bash_bg: rgb(0x413C41),
    success: rgb(0x4EBA65),
    error: rgb(0xFF6B80),
    auto_mode: rgb(0xFFC107),
    plan_mode: rgb(0x48968C),
    accept_edits: rgb(0xAF87FF),
    diff_text: rgb(0xF8F8F2),
    diff_add: rgb(0x50C850),
    diff_add_bg: rgb(0x022800),
    diff_del: rgb(0xDC5A5A),
    diff_del_bg: rgb(0x3D0100),
    code_keyword: Color::Blue,
    code_string: Color::Red,
    code_macro: Color::Cyan,
};

/// Nearest xterm-256 entries of `CC_DARK`.
const CC_DARK_256: Cc = Cc {
    claude: Color::Indexed(173),
    claude_shimmer: Color::Indexed(216),
    permission: Color::Indexed(147),
    inactive: Color::Indexed(246),
    inactive_shimmer: Color::Indexed(250),
    subtle: Color::Indexed(239),
    prompt_border: Color::Indexed(244),
    text: Color::Indexed(231),
    user_bg: Color::Indexed(237),
    bash: Color::Indexed(205),
    bash_bg: Color::Indexed(237),
    success: Color::Indexed(71),
    error: Color::Indexed(204),
    auto_mode: Color::Indexed(214),
    plan_mode: Color::Indexed(72),
    accept_edits: Color::Indexed(141),
    diff_text: Color::Indexed(255),
    diff_add: Color::Indexed(77),
    diff_add_bg: Color::Indexed(22),
    diff_del: Color::Indexed(167),
    diff_del_bg: Color::Indexed(52),
    code_keyword: Color::Blue,
    code_string: Color::Red,
    code_macro: Color::Cyan,
};

/// Claude Code's light theme as documented (not captured; see Intentional
/// Differences in the UI plan).
const CC_LIGHT: Cc = Cc {
    claude: rgb(0xD77757),
    claude_shimmer: rgb(0xF5A788),
    permission: rgb(0x5769F7),
    inactive: rgb(0x666666),
    inactive_shimmer: rgb(0x8A8A8A),
    subtle: rgb(0xAFAFAF),
    prompt_border: rgb(0x999999),
    text: rgb(0x000000),
    user_bg: rgb(0xF0F0F0),
    bash: rgb(0xFF0087),
    bash_bg: rgb(0xFCE4EC),
    success: rgb(0x2C7A39),
    error: rgb(0xAB2B3F),
    auto_mode: rgb(0x966C1E),
    plan_mode: rgb(0x006666),
    accept_edits: rgb(0x8700FF),
    diff_text: rgb(0x1C1C1C),
    diff_add: rgb(0x2C7A39),
    diff_add_bg: rgb(0xDAFBE1),
    diff_del: rgb(0xAB2B3F),
    diff_del_bg: rgb(0xFFEBE9),
    code_keyword: Color::Blue,
    code_string: Color::Red,
    code_macro: Color::Cyan,
};

const CC_LIGHT_256: Cc = Cc {
    claude: Color::Indexed(173),
    claude_shimmer: Color::Indexed(216),
    permission: Color::Indexed(63),
    inactive: Color::Indexed(241),
    inactive_shimmer: Color::Indexed(245),
    subtle: Color::Indexed(145),
    prompt_border: Color::Indexed(246),
    text: Color::Indexed(16),
    user_bg: Color::Indexed(255),
    bash: Color::Indexed(198),
    bash_bg: Color::Indexed(225),
    success: Color::Indexed(28),
    error: Color::Indexed(124),
    auto_mode: Color::Indexed(136),
    plan_mode: Color::Indexed(23),
    accept_edits: Color::Indexed(93),
    diff_text: Color::Indexed(234),
    diff_add: Color::Indexed(28),
    diff_add_bg: Color::Indexed(194),
    diff_del: Color::Indexed(124),
    diff_del_bg: Color::Indexed(224),
    code_keyword: Color::Blue,
    code_string: Color::Red,
    code_macro: Color::Cyan,
};

/// Eight named colors. Backgrounds are dropped; glyphs carry the meaning.
const CC_BASIC: Cc = Cc {
    claude: Color::LightRed,
    claude_shimmer: Color::LightRed,
    permission: Color::LightBlue,
    inactive: Color::Gray,
    inactive_shimmer: Color::Gray,
    subtle: Color::DarkGray,
    prompt_border: Color::DarkGray,
    text: Color::White,
    user_bg: Color::Reset,
    bash: Color::LightMagenta,
    bash_bg: Color::Reset,
    success: Color::Green,
    error: Color::Red,
    auto_mode: Color::Yellow,
    plan_mode: Color::Cyan,
    accept_edits: Color::Magenta,
    diff_text: Color::White,
    diff_add: Color::Green,
    diff_add_bg: Color::Reset,
    diff_del: Color::Red,
    diff_del_bg: Color::Reset,
    code_keyword: Color::Blue,
    code_string: Color::Red,
    code_macro: Color::Cyan,
};

const CC_NONE: Cc = Cc {
    claude: Color::Reset,
    claude_shimmer: Color::Reset,
    permission: Color::Reset,
    inactive: Color::Reset,
    inactive_shimmer: Color::Reset,
    subtle: Color::Reset,
    prompt_border: Color::Reset,
    text: Color::Reset,
    user_bg: Color::Reset,
    bash: Color::Reset,
    bash_bg: Color::Reset,
    success: Color::Reset,
    error: Color::Reset,
    auto_mode: Color::Reset,
    plan_mode: Color::Reset,
    accept_edits: Color::Reset,
    diff_text: Color::Reset,
    diff_add: Color::Reset,
    diff_add_bg: Color::Reset,
    diff_del: Color::Reset,
    diff_del_bg: Color::Reset,
    code_keyword: Color::Reset,
    code_string: Color::Reset,
    code_macro: Color::Reset,
};
```

Add to `impl Theme`:

```rust
    /// The Claude Code palette in this theme's encoding. Behind a modal every
    /// accent drops to `inactive` and backgrounds go, as the ramp does.
    pub fn cc(&self) -> Cc {
        if self.no_color {
            return CC_NONE;
        }
        let base = match (self.depth_hint(), self.is_light()) {
            (ColorDepth::TrueColor, false) => CC_DARK,
            (ColorDepth::TrueColor, true) => CC_LIGHT,
            (ColorDepth::Ansi256, false) => CC_DARK_256,
            (ColorDepth::Ansi256, true) => CC_LIGHT_256,
            (ColorDepth::Basic, _) => CC_BASIC,
        };
        if !self.dimmed {
            return base;
        }
        let quiet = base.inactive;
        Cc {
            claude: quiet,
            claude_shimmer: quiet,
            permission: quiet,
            inactive_shimmer: quiet,
            text: quiet,
            bash: quiet,
            success: quiet,
            error: quiet,
            auto_mode: quiet,
            plan_mode: quiet,
            accept_edits: quiet,
            diff_add: quiet,
            diff_del: quiet,
            user_bg: Color::Reset,
            bash_bg: Color::Reset,
            diff_add_bg: Color::Reset,
            diff_del_bg: Color::Reset,
            ..base
        }
    }
```

Note: `Theme::dim()` builds its dark result from `TRUECOLOR_DIM`, whose `primary` is still `Color::Rgb`, so `depth_hint()` stays correct.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-tui --offline cc_palette`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```powershell
git add crates/davinci-tui/src/davinci/theme.rs
git commit -m "feat(tui): add the Claude Code palette to the theme"
```

### Task A2: Enter runs the highlighted slash command

**Files:**
- Modify: `crates/davinci-tui/src/davinci/app.rs` (`handle_suggestion_key`, around line 783)
- Test: `crates/davinci-tui/src/davinci/app.rs`, `mod section_regressions`

**Interfaces:**
- Consumes: `Model::accept_suggestion() -> bool`, `Model::submit()`, `Flow::Submit(String)`.
- Produces: `fn runs_on_enter(model: &Model) -> bool` (private).

- [ ] **Step 1: Write the failing tests** (in `mod section_regressions`)

```rust
    fn slash_model() -> Model {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, false);
        model.slash_commands = vec![
            crate::autocomplete::SlashCommandSpec {
                name: "compact".into(),
                description: "Compact the session context".into(),
                ..Default::default()
            },
            crate::autocomplete::SlashCommandSpec {
                name: "config".into(),
                ..Default::default()
            },
        ];
        model
    }

    #[test]
    fn enter_on_a_partial_slash_command_runs_the_highlighted_command() {
        let mut model = slash_model();
        model.composer.set_text("/comp");
        model.refresh_suggestions();
        assert_eq!(
            handle_key(&mut model, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Flow::Submit("/compact".into())
        );
        assert_eq!(model.composer.to_string(), "");
        assert!(model.suggestions.is_none());
    }

    #[test]
    fn tab_on_a_partial_slash_command_completes_without_running() {
        let mut model = slash_model();
        model.composer.set_text("/comp");
        model.refresh_suggestions();
        assert_eq!(
            handle_key(&mut model, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Flow::Continue
        );
        assert_eq!(model.composer.to_string(), "/compact ");
    }

    #[test]
    fn enter_on_a_slash_argument_runs_the_command_with_it() {
        let mut model = slash_model();
        model.slash_commands.push(crate::autocomplete::SlashCommandSpec {
            name: "thinking".into(),
            ..Default::default()
        });
        model.thinking_levels = vec!["low".into(), "high".into()];
        model.composer.set_text("/thinking hi");
        model.refresh_suggestions();
        assert!(model.suggestions.is_some(), "thinking levels are offered");
        assert_eq!(
            handle_key(&mut model, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Flow::Submit("/thinking high".into())
        );
    }

    #[test]
    fn enter_on_a_file_suggestion_inserts_the_path_without_sending() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "x").unwrap();
        let mut model = slash_model();
        model.cwd = dir.path().display().to_string();
        model.composer.set_text("look at @READ");
        model.refresh_suggestions();
        assert!(model.suggestions.is_some(), "the file is offered");
        assert_eq!(
            handle_key(&mut model, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Flow::Continue
        );
        assert_eq!(model.composer.to_string(), "look at @README.md ");
    }
```

If `model.thinking_levels` or `model.cwd` has another name, use the field that `refresh_suggestions` passes as `thinking_levels` and `cwd` in `SuggestionQuery` (model.rs, `refresh_suggestions`).

- [ ] **Step 2: Run them**

Run: `rtk cargo test -p davinci-tui --offline enter_on_a_ tab_on_a_partial`
Expected: `enter_on_a_partial_slash_command_runs_the_highlighted_command` and `enter_on_a_slash_argument_runs_the_command_with_it` FAIL (`Continue` instead of `Submit`). The other two pass already; they guard behavior that must not change.

- [ ] **Step 3: Implement.** Replace the block that starts with `if tab || bindings.matches(data, "tui.input.submit") {` (the second one, after the `/model` special case) with:

```rust
    if tab || bindings.matches(data, "tui.input.submit") {
        // Claude Code: Enter on a slash list runs the highlighted row
        // (`/comp` + Enter runs `/compact`); Tab only completes. File and
        // extension lists complete on both keys.
        let run = !tab && runs_on_enter(model);
        if model.accept_suggestion() || tab {
            if run {
                let sent = model.composer.editor().get_expanded_text().trim_end().to_string();
                model.dismiss_suggestions();
                model.submit();
                return Some(Flow::Submit(sent));
            }
            return Some(Flow::Continue);
        }
        // Enter on a row the composer already holds sends it instead.
        return None;
    }
```

Add below `handle_suggestion_key`:

```rust
/// Whether Enter on the open list runs the command: only for a slash command
/// or its argument, never for an `@` path or an extension token.
fn runs_on_enter(model: &Model) -> bool {
    model.composer.trim_start().starts_with('/')
        && model
            .suggestions
            .as_ref()
            .is_some_and(|found| !found.prefix.starts_with(['@', '#']))
}
```

Update the comment above the old block ("Tab and enter both take the marked row…") to describe the new rule.

- [ ] **Step 4: Run the crate tests**

Run: `rtk cargo test -p davinci-tui --offline`
Expected: the 4 new tests pass. If an older test expects Enter on a slash row to only complete, it pins the old behavior: change its expectation to `Flow::Submit(<completed command>)`.

- [ ] **Step 5: Commit**

```powershell
git add crates/davinci-tui/src/davinci/app.rs
git commit -m "feat(tui): run the highlighted slash command on Enter"
```

### Task A3: Completion list layout

**Files:**
- Modify: `crates/davinci-tui/src/davinci/views/chrome.rs` (`suggestions`, `suggestions_height`, the `SELECTION_BAR`/`UNSELECTED_BAR` uses in these two functions only)
- Test: `crates/davinci-tui/src/davinci/views/chrome.rs` tests module

**Interfaces:**
- Produces: `pub const SUGGESTION_ROWS: usize = 5;`, `fn suggestion_item_rows(label: &str, description: Option<&str>, name_column: u16, width: u16) -> Vec<(String, String)>`, `fn visible_items(heights: &[usize], selected: usize, max_rows: usize) -> (usize, usize)`.

- [ ] **Step 1: Write the failing tests** (chrome.rs tests module; reuse its model helper if one exists, otherwise add this one)

```rust
    fn listing(commands: &[(&str, &str)], typed: &str) -> Model {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, false);
        m.slash_commands = commands
            .iter()
            .map(|(name, description)| crate::autocomplete::SlashCommandSpec {
                name: (*name).into(),
                description: (*description).into(),
                ..Default::default()
            })
            .collect();
        m.composer.set_text(typed);
        m.refresh_suggestions();
        m
    }

    #[test]
    fn a_command_row_matches_the_reference_columns() {
        let m = listing(
            &[
                ("compact", "Free up context by summarizing the conversation so far"),
                ("competitive", "Short"),
            ],
            "/comp",
        );
        let rows = suggestions(&m);
        let first = rows[0].to_string();
        assert!(first.starts_with("  /compact "), "{first}");
        assert_eq!(first.find("Free"), Some(50), "{first}");
        let cc = m.theme.cc();
        for span in rows[0].spans.iter().filter(|s| !s.content.trim().is_empty()) {
            assert_eq!(span.style.fg, Some(cc.permission), "{:?}", span.content);
        }
        for span in rows[1].spans.iter().filter(|s| !s.content.trim().is_empty()) {
            assert_eq!(span.style.fg, Some(cc.inactive), "{:?}", span.content);
        }
        assert!(!rows.iter().any(|r| r.to_string().contains("commands ·")), "no footer");
        assert!(!rows[0].to_string().trim().is_empty(), "no leading blank row");
    }

    #[test]
    fn the_list_shows_at_most_five_rows_of_whole_items() {
        let many: Vec<(String, String)> =
            (0..12).map(|i| (format!("cmd{i:02}"), "d".to_string())).collect();
        let refs: Vec<(&str, &str)> = many.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let m = listing(&refs, "/cmd");
        assert_eq!(suggestions(&m).len(), 5);
        assert_eq!(suggestions_height(&m), 5);
    }

    #[test]
    fn a_long_description_wraps_to_two_rows_and_ends_in_an_ellipsis() {
        let long = "word ".repeat(60);
        let m = listing(&[("writing-plans", long.as_str())], "/wri");
        let rows = suggestions(&m);
        assert_eq!(rows.len(), 2);
        assert!(rows[1].to_string().trim_end().ends_with('…'));
        assert_eq!(rows[1].to_string().find("word"), Some(50));
    }

    #[test]
    fn the_selected_item_stays_visible_when_scrolled() {
        let many: Vec<(String, String)> =
            (0..12).map(|i| (format!("cmd{i:02}"), "d".to_string())).collect();
        let refs: Vec<(&str, &str)> = many.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let mut m = listing(&refs, "/cmd");
        m.suggestion_index = 9;
        let text: Vec<String> = suggestions(&m).iter().map(|r| r.to_string()).collect();
        assert!(text.iter().any(|r| r.starts_with("  /cmd09")), "{text:?}");
    }

    #[test]
    fn visible_items_fill_whole_items_around_the_selection() {
        assert_eq!(visible_items(&[2, 2, 2, 2], 0, 5), (0, 2));
        assert_eq!(visible_items(&[2, 2, 2, 2], 3, 5), (2, 4));
        assert_eq!(visible_items(&[1, 1, 1, 1, 1, 1, 1], 6, 5), (2, 7));
        assert_eq!(visible_items(&[], 0, 5), (0, 0));
    }

    #[test]
    fn a_file_row_reads_plus_then_the_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "x").unwrap();
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, false);
        m.cwd = dir.path().display().to_string();
        m.composer.set_text("@READ");
        m.refresh_suggestions();
        let first = suggestions(&m)[0].to_string();
        assert!(first.starts_with("  + README.md"), "{first}");
    }
```

- [ ] **Step 2: Run them**

Run: `rtk cargo test -p davinci-tui --offline a_command_row the_list_shows a_long_description the_selected_item_stays visible_items a_file_row`
Expected: FAIL (old layout: selection bar, footer, blank row).

- [ ] **Step 3: Implement.** Replace `suggestions` and `suggestions_height` in `chrome.rs` with:

```rust
/// Rows the completion list may take, as in Claude Code.
pub const SUGGESTION_ROWS: usize = 5;

/// What the composer is offering, drawn directly above it with no border,
/// counter or footer (docs/ui/claude-code-reference/ui/03-slash-all). The
/// selected item is `permission`, the rest `inactive`.
pub fn suggestions(model: &Model) -> Vec<Line<'static>> {
    if let Some(rows) = super::cogitator::suggestions(model) {
        return rows;
    }
    let Some(found) = &model.suggestions else {
        return Vec::new();
    };
    if found.items.is_empty() {
        return Vec::new();
    }
    let cc = model.theme.cc();
    let name_column = ((model.width as usize * 2 / 5).max(20)) as u16;
    let items: Vec<Vec<(String, String)>> = found
        .items
        .iter()
        .map(|item| {
            let label = suggestion_label(&found.prefix, model, &item.label);
            suggestion_item_rows(&label, item.description.as_deref(), name_column, model.width)
        })
        .collect();
    let heights: Vec<usize> = items.iter().map(Vec::len).collect();
    let selected = model.suggestion_index.min(items.len() - 1);
    let (start, end) = visible_items(&heights, selected, SUGGESTION_ROWS);
    let mut out = Vec::new();
    for (index, rows) in items.iter().enumerate().take(end).skip(start) {
        let color = if index == selected { cc.permission } else { cc.inactive };
        for (left, description) in rows {
            let mut spans = vec![span(left.clone(), color)];
            if !description.is_empty() {
                spans.push(span(description.clone(), color));
            }
            out.push(Line::from(crate::davinci::ui::truncate_run(spans, model.width)));
        }
    }
    out
}

/// How many rows [`suggestions`] will occupy, known before it is built.
pub fn suggestions_height(model: &Model) -> u16 {
    suggestions(model).len() as u16
}

/// `/name` for a command, `+ path` for a file, the bare value otherwise.
fn suggestion_label(prefix: &str, model: &Model, label: &str) -> String {
    if prefix.starts_with('@') {
        let path = label.trim_start_matches('@');
        return format!("+ {}", path.replace('/', std::path::MAIN_SEPARATOR_STR));
    }
    let naming_a_command = prefix.starts_with('/') && !model.composer.contains(' ');
    if naming_a_command {
        return format!("/{}", label.trim_start_matches('/'));
    }
    label.to_string()
}

/// One item as `(left, description)` rows: the label padded to the name
/// column on the first row, then at most two description rows.
fn suggestion_item_rows(
    label: &str,
    description: Option<&str>,
    name_column: u16,
    width: u16,
) -> Vec<(String, String)> {
    let label = clip_ellipsis(label, name_column.saturating_sub(2));
    let description = description.map(str::trim).filter(|d| !d.is_empty());
    let Some(description) = description else {
        return vec![(format!("  {label}"), String::new())];
    };
    let room = width.saturating_sub(2 + name_column + 2).max(8);
    let mut wrapped = crate::davinci::ui::wrap(description, room);
    if wrapped.len() > 2 {
        let rest = wrapped[1..].join(" ");
        wrapped.truncate(1);
        wrapped.push(rest);
    }
    if let Some(second) = wrapped.get_mut(1) {
        if UnicodeWidthStr::width(second.as_str()) > room as usize {
            *second = clip_ellipsis(second, room);
        }
    }
    let pad = |text: &str| {
        let used = UnicodeWidthStr::width(text);
        format!("{text}{}", " ".repeat((2 + name_column as usize).saturating_sub(used)))
    };
    let mut rows = vec![(pad(&format!("  {label}")), wrapped[0].clone())];
    if let Some(second) = wrapped.get(1) {
        rows.push((pad(""), second.clone()));
    }
    rows
}

/// The `[start, end)` range of items to draw: whole items only, at most
/// `max_rows` rows, the selected item always included.
fn visible_items(heights: &[usize], selected: usize, max_rows: usize) -> (usize, usize) {
    if heights.is_empty() {
        return (0, 0);
    }
    let selected = selected.min(heights.len() - 1);
    let mut start = 0;
    while start < selected && heights[start..=selected].iter().sum::<usize>() > max_rows {
        start += 1;
    }
    let mut end = selected + 1;
    while end < heights.len() && heights[start..=end].iter().sum::<usize>() <= max_rows {
        end += 1;
    }
    (start, end)
}
```

Check `clip_ellipsis` adds `…` only when it cuts: the wrap test expects `…` on the second row when the description continues past it.

After the rewrite nothing in `chrome.rs` uses `span_on`, the `SELECTION_BAR` import or the private `UNSELECTED_BAR` constant (chrome.rs lines 16, 20, 25). Delete all three, or clippy `-D warnings` fails.

In `app.rs::compose` the list is already pushed directly before the composer rows (`rows.extend(offered)`), so no layout change is needed there.

- [ ] **Step 4: Run the crate tests**

Run: `rtk cargo test -p davinci-tui --offline`
Expected: new tests pass. Update older tests that assert `… N below`, `commands · ↑↓` or the selection bar in the composer list (see "Tests That Will Change").

- [ ] **Step 5: Commit**

```powershell
git add crates/davinci-tui/src/davinci/views/chrome.rs
git commit -m "feat(tui): draw the completion list like Claude Code"
```

### Task A4: Composer, placeholder, shell mode and effort line

**Files:**
- Modify: `crates/davinci-tui/src/davinci/views/chrome.rs` (`composer`, `composer_rule` callers, `effort_line`)
- Modify: `crates/davinci-tui/src/davinci/model.rs` (add `pub placeholder: usize` to `Model`, initialized to `0` in `Model::new`)
- Test: chrome.rs tests module

**Interfaces:**
- Produces: `pub const PLACEHOLDERS: [&str; 3]`, `fn recolor_leading(spans: Vec<Span<'static>>, cells: usize, color: Color) -> Vec<Span<'static>>`.

- [ ] **Step 1: Write the failing tests**

```rust
    fn composing(text: &str) -> (Model, Vec<Line<'static>>) {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, false);
        m.slash_commands = vec![crate::autocomplete::SlashCommandSpec {
            name: "model".into(),
            argument_hint: Some("[model]".into()),
            ..Default::default()
        }];
        m.composer.set_text(text);
        let rows = composer(&m, None, Hint::None);
        (m, rows)
    }

    #[test]
    fn an_empty_composer_invites_with_a_dim_example() {
        let (m, rows) = composing("");
        let input = &rows[2];
        assert!(input.to_string().starts_with("❯\u{a0}Try \"fix lint errors\""), "{input}");
        let hint = input.spans.iter().find(|s| s.content.contains("Try")).unwrap();
        assert!(hint.style.add_modifier.contains(Modifier::DIM));
        assert_eq!(rows[1].spans[0].style.fg, Some(m.theme.cc().prompt_border));
    }

    #[test]
    fn a_bang_draft_switches_the_composer_to_shell_mode() {
        let (m, rows) = composing("!git status");
        let cc = m.theme.cc();
        assert!(rows[2].to_string().starts_with("!\u{a0}git status"), "{}", rows[2]);
        assert_eq!(rows[2].spans[0].style.fg, Some(cc.bash));
        assert_eq!(rows[1].spans[0].style.fg, Some(cc.bash));
        assert_eq!(rows[3].spans[0].style.fg, Some(cc.bash));
    }

    #[test]
    fn a_known_slash_command_is_drawn_in_the_permission_color() {
        let (m, rows) = composing("/model");
        let command = rows[2].spans.iter().find(|s| s.content.contains("/model")).unwrap();
        assert_eq!(command.style.fg, Some(m.theme.cc().permission));
    }

    #[test]
    fn a_completed_command_shows_its_argument_hint() {
        let (m, rows) = composing("/model ");
        let hint = rows[2].spans.iter().find(|s| s.content.contains("[model]")).unwrap();
        assert_eq!(hint.style.fg, Some(m.theme.cc().inactive));
    }

    #[test]
    fn the_effort_line_is_inactive_throughout() {
        let (m, rows) = composing("");
        for span in rows[0].spans.iter().filter(|s| !s.content.trim().is_empty()) {
            assert_eq!(span.style.fg, Some(m.theme.cc().inactive), "{:?}", span.content);
        }
    }
```

Add `use ratatui::style::Modifier;` to the tests module if it is not imported.

- [ ] **Step 2: Run them**

Run: `rtk cargo test -p davinci-tui --offline an_empty_composer a_bang_draft a_known_slash a_completed_command the_effort_line`
Expected: FAIL.

- [ ] **Step 3: Implement** in `chrome.rs`:

1. Add near the top:

```rust
/// Examples shown in an empty conversation composer, one per session.
pub const PLACEHOLDERS: [&str; 3] = ["fix lint errors", "fix typecheck errors", "refactor <filepath>"];
```

2. `effort_line`: color both spans `model.theme.cc().inactive`:

```rust
pub fn effort_line(model: &Model) -> Line<'static> {
    let quiet = model.theme.cc().inactive;
    spread(
        model.width,
        Vec::new(),
        vec![
            span("● ", quiet),
            span(model.thinking_level.to_lowercase(), quiet),
            span(" · /effort  ", quiet),
        ],
    )
}
```

3. In `composer`, after `let th = &model.theme;` add:

```rust
    let cc = th.cc();
    let shell_mode = model.screen == Screen::Agent && model.composer.starts_with('!');
```

Set `let border = if shell_mode { cc.bash } else { cc.prompt_border };` in place of `let border = th.border;`. In `composer_rule`, the `hidden == 0` branch returns `print_rule(model.width, &model.theme)`, which ignores the color it was given; replace that line with `return Line::from(span("─".repeat(model.width as usize), color));` so both rules take `border`, and remove the now unused `print_rule` import. Change the style import at the top of chrome.rs to `use ratatui::style::{Color, Modifier, Style};` (the new helpers use `Color` and `Modifier`).

4. Placeholder: after the existing `placeholder` computation, fall back to the example on the conversation screen:

```rust
    let placeholder = placeholder.or_else(|| {
        (model.screen == Screen::Agent && lines.is_none())
            .then(|| format!("Try \"{}\"", PLACEHOLDERS[model.placeholder % PLACEHOLDERS.len()]))
    });
```

and draw it dim: in the `entry.is_empty()` branch use

```rust
            let mut hint = span(placeholder.clone().unwrap_or_default(), th.text);
            hint.style = hint.style.add_modifier(Modifier::DIM);
            vec![hint]
```

(import `ratatui::style::Modifier` in chrome.rs if needed).

5. Text color: replace the `ink` computation (`if entry.starts_with('/') { th.muted } else { th.text }`) with `let ink = th.text;`.

6. Shell mode on the first row: before `composer_view`, strip the `!` from row 0 and shift the caret:

```rust
        let (entry, column) = if shell_mode && index == 0 {
            let rest = entry.strip_prefix('!').unwrap_or(&entry).to_string();
            (rest, column.map(|col| col.saturating_sub(1)))
        } else {
            (entry, column)
        };
```

(declare `column` before this block; keep the existing `let column = caret_at.filter(…)` line above it.)

7. Prompt: replace the prompt construction with

```rust
        let prompt = if index == start {
            let glyph_text = if shell_mode { "!" } else { glyph::PROMPT };
            format!("{glyph_text}\u{a0}")
        } else {
            "  ".into()
        };
        let prompt_color = if overlaid {
            th.border
        } else if shell_mode {
            cc.bash
        } else if model.running {
            cc.inactive
        } else {
            th.text
        };
        let mut row = vec![span(prompt, prompt_color)];
```

8. Slash command color and argument hint, just before `rows.push(Line::from(… truncate_run(run …)))`:

```rust
        let mut run = row;
        if index == 0 && !shell_mode {
            if let Some(name) = known_command(model, &entry) {
                run = recolor_leading(run, 2 + 1 + name.chars().count(), cc.permission);
                let spec = model.slash_commands.iter().find(|spec| spec.name == name);
                if entry == format!("/{name} ") {
                    if let Some(hint) = spec.and_then(|spec| spec.argument_hint.clone()) {
                        run.push(span(hint, cc.inactive));
                    }
                }
            }
        }
```

with helpers:

```rust
/// The command a draft starts with, when it names a known slash command.
fn known_command(model: &Model, entry: &str) -> Option<String> {
    let word = entry.strip_prefix('/')?.split_whitespace().next()?;
    model
        .slash_commands
        .iter()
        .any(|spec| spec.name == word)
        .then(|| word.to_string())
}

/// Recolor the first `cells` display cells of a row, splitting a span at the
/// boundary. The prompt's two cells count; backgrounds (the caret) are kept.
fn recolor_leading(spans: Vec<Span<'static>>, cells: usize, color: Color) -> Vec<Span<'static>> {
    let mut left = cells;
    let mut out = Vec::with_capacity(spans.len() + 1);
    for (index, mut span) in spans.into_iter().enumerate() {
        if index == 0 || left == 0 {
            left = left.saturating_sub(UnicodeWidthStr::width(span.content.as_ref()));
            out.push(span);
            continue;
        }
        let text = span.content.to_string();
        let width = UnicodeWidthStr::width(text.as_str());
        if width <= left {
            left -= width;
            span.style = span.style.fg(color);
            out.push(span);
            continue;
        }
        let mut split = 0;
        let mut used = 0;
        for (at, ch) in text.char_indices() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + w > left {
                split = at;
                break;
            }
            used += w;
            split = at + ch.len_utf8();
        }
        let (head, tail) = text.split_at(split);
        out.push(Span::styled(head.to_string(), span.style.fg(color)));
        out.push(Span::styled(tail.to_string(), span.style));
        left = 0;
    }
    out
}
```

(The prompt span is skipped by `index == 0`; `cells` counts it so that `2 + 1 + name` covers `❯ /name`.)

9. In `model.rs` add `pub placeholder: usize,` to `Model` with a doc comment `/// Which empty-composer example this session shows.` and `placeholder: 0,` in `Model::new`. In `crates/davinci-coding-agent/src/davinci_interactive.rs`, right after `let mut model = davinci_tui::davinci::boot(raw, 100, 44);` (around line 4073), set `model.placeholder = (std::process::id() as usize) % 3;`.

- [ ] **Step 4: Run the crate tests**

Run: `rtk cargo test -p davinci-tui --offline`
Expected: new tests pass. Existing tests that assert an empty composer shows no text, or that `/…` text is muted, pin the old look: update them.

- [ ] **Step 5: Commit**

```powershell
git add crates/davinci-tui crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "feat(tui): Claude Code composer, placeholder and shell mode"
```

### Task A5: Footer and the `?` shortcuts panel

**Files:**
- Modify: `crates/davinci-tui/src/davinci/views/chrome.rs` (`conversation_status`, new `footer`, new `shortcut_rows`)
- Modify: `crates/davinci-tui/src/davinci/app.rs` (`compose`: footer rows; `handle_key`: `?` toggle)
- Modify: `crates/davinci-tui/src/davinci/model.rs` (`pub shortcuts_open: bool`, default `false`)
- Test: chrome.rs and app.rs tests

**Interfaces:**
- Produces: `pub fn footer(model: &Model) -> Vec<Line<'static>>`; `fn shortcut_rows(model: &Model) -> Vec<Line<'static>>`; `fn key_label(model: &Model, action: &str) -> Option<String>`.

- [ ] **Step 1: Write the failing tests** (chrome.rs tests)

```rust
    fn footer_for(mode: &str, running: bool) -> Line<'static> {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, false);
        m.permission_mode = mode.into();
        m.running = running;
        footer(&m).remove(0)
    }

    #[test]
    fn the_footer_names_each_mode_like_claude_code() {
        let cc = Theme::da_vinci(ColorDepth::TrueColor, false).cc();
        let cases = [
            ("ask", "  ⏸ manual mode on · ? for shortcuts", cc.inactive),
            ("edits", "  ⏵⏵ accept edits on (shift+tab to cycle)", cc.accept_edits),
            ("read-only", "  ⏸ plan mode on (shift+tab to cycle)", cc.plan_mode),
            ("auto", "  ⏵⏵ auto mode on (shift+tab to cycle)", cc.auto_mode),
            ("always-approve", "  ⏵⏵ always approve on · no prompts (shift+tab to cycle)", cc.error),
        ];
        for (mode, text, color) in cases {
            let line = footer_for(mode, false);
            assert_eq!(line.to_string().trim_end(), text, "{mode}");
            let first = line.spans.iter().find(|s| !s.content.trim().is_empty()).unwrap();
            assert_eq!(first.style.fg, Some(color), "{mode}");
        }
    }

    #[test]
    fn a_running_turn_puts_esc_to_interrupt_in_the_footer() {
        assert_eq!(
            footer_for("ask", true).to_string().trim_end(),
            "  ⏸ manual mode on · esc to interrupt"
        );
        assert_eq!(
            footer_for("auto", true).to_string().trim_end(),
            "  ⏵⏵ auto mode on (shift+tab to cycle) · esc to interrupt"
        );
    }

    #[test]
    fn shell_mode_owns_the_footer() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, false);
        m.composer.set_text("!ls");
        let line = footer(&m).remove(0);
        assert_eq!(line.to_string().trim_end(), "  ! for shell mode");
        assert_eq!(line.spans[1].style.fg, Some(m.theme.cc().bash));
    }

    #[test]
    fn the_shortcuts_panel_uses_three_reference_columns() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, false);
        m.shortcuts_open = true;
        let rows = footer(&m);
        assert!(rows.len() >= 3);
        let first = rows[0].to_string();
        assert!(first.starts_with("  ! for shell mode"), "{first}");
        assert_eq!(first.find("shift + tab"), Some(26), "{first}");
        assert!(rows[1].to_string().starts_with("  / for commands"));
        assert!(rows[2].to_string().starts_with("  @ for file paths"));
    }
```

And in app.rs `mod section_regressions`:

```rust
    #[test]
    fn question_mark_in_an_empty_composer_toggles_the_shortcuts_panel() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, false);
        handle_key(&mut model, KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(model.shortcuts_open);
        assert_eq!(model.composer.to_string(), "");
        handle_key(&mut model, KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(!model.shortcuts_open);
        assert_eq!(model.composer.to_string(), "a");
        handle_key(&mut model, KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert_eq!(model.composer.to_string(), "a?", "? is text once the draft has text");
    }
```

- [ ] **Step 2: Run them**

Run: `rtk cargo test -p davinci-tui --offline the_footer_names a_running_turn_puts shell_mode_owns the_shortcuts_panel question_mark_in`
Expected: FAIL.

- [ ] **Step 3: Implement.**

`model.rs`: add `pub shortcuts_open: bool,` (doc: `/// \`?\` in an empty composer: the shortcuts panel replaces the footer.`) and `shortcuts_open: false,` in `Model::new`.

`chrome.rs`: replace `conversation_status` with:

```rust
/// The footer under the composer (docs/ui/claude-code-reference/ui/14-mode-1
/// to 18-mode-5). The mode word keeps a glyph, so it reads without color.
fn conversation_status(model: &Model) -> Line<'static> {
    let th = &model.theme;
    let cc = th.cc();
    if model.exit_armed {
        return Line::from(span("  Press Ctrl-C again to exit", cc.inactive));
    }
    if model.screen == Screen::Agent && model.composer.starts_with('!') {
        return Line::from(vec![span("  ", cc.inactive), span("! for shell mode", cc.bash)]);
    }
    let cycle = key_label(model, "app.permissions.cycle").unwrap_or_else(|| "shift+tab".into());
    let cycle_note = format!(" ({cycle} to cycle)");
    let (label, color, tail) = match model.permission_mode.as_str() {
        "edits" => ("⏵⏵ accept edits on", cc.accept_edits, cycle_note),
        "read-only" => ("⏸ plan mode on", cc.plan_mode, cycle_note),
        "auto" => ("⏵⏵ auto mode on", cc.auto_mode, cycle_note),
        "always-approve" => ("⏵⏵ always approve on · no prompts", cc.error, cycle_note),
        _ => ("⏸ manual mode on", cc.inactive, String::new()),
    };
    let mut left = vec![span("  ", cc.inactive), span(label, color)];
    if !tail.is_empty() {
        left.push(span(tail, cc.inactive));
    }
    let manual = tail_is_manual(&model.permission_mode);
    if model.running {
        left.push(span(" · esc to interrupt", cc.inactive));
    } else if manual {
        left.push(span(" · ? for shortcuts", cc.inactive));
    }
    let agents = model.agents.as_ref().map_or(0, |sheet| sheet.agents.len());
    if agents > 0 {
        let noun = if agents == 1 { "agent" } else { "agents" };
        left.push(span(format!(" · ← {agents} {noun}"), cc.inactive));
    }
    if !model.voice.notice.is_empty() {
        left.push(span(format!(" · {}", model.voice.notice), cc.inactive));
    }
    if let Some(jobs) = jobs_note(model) {
        left.push(span(format!(" · {jobs}"), cc.inactive));
    }
    let right = if model.context_fraction() >= 0.8 && model.width >= 80 {
        vec![span(
            format!("{}% context ", (model.context_fraction() * 100.0) as u32),
            cc.auto_mode,
        )]
    } else {
        Vec::new()
    };
    spread(model.width, left, right)
}

fn tail_is_manual(mode: &str) -> bool {
    !matches!(mode, "edits" | "read-only" | "auto" | "always-approve")
}

/// A binding as the shortcuts panel spells it: `ctrl+o` becomes `ctrl + o`.
fn key_label(model: &Model, action: &str) -> Option<String> {
    model
        .keybindings
        .keys_for(action)
        .first()
        .map(|key| key.to_string())
}

/// The rows under the composer: the footer, or the shortcuts panel.
pub fn footer(model: &Model) -> Vec<Line<'static>> {
    if model.shortcuts_open && model.screen == Screen::Agent && model.overlay.is_none() {
        return shortcut_rows(model);
    }
    vec![status(model)]
}

/// Claude Code's `?` panel (ui/02-shortcuts), filled with davinci's actual
/// bindings. Columns start at 2, 26 and 61; below 100 columns they stack.
fn shortcut_rows(model: &Model) -> Vec<Line<'static>> {
    let quiet = model.theme.cc().inactive;
    let spaced = |action: &str, text: &str| {
        key_label(model, action).map(|key| format!("{} {text}", key.replace('+', " + ")))
    };
    let columns: [Vec<String>; 3] = [
        vec![
            "! for shell mode".into(),
            "/ for commands".into(),
            "@ for file paths".into(),
        ],
        [
            spaced("app.permissions.cycle", "to cycle modes"),
            spaced("davinci.tools.expand", "for verbose output"),
            spaced("davinci.composer.newLine", "for newline"),
        ]
        .into_iter()
        .flatten()
        .collect(),
        [
            spaced("app.model.select", "to switch model"),
            spaced("app.editor.external", "to edit in $EDITOR"),
            Some("/hotkeys to customize".to_string()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    ];
    if model.width < 100 {
        return columns
            .iter()
            .flatten()
            .map(|text| Line::from(span(format!("  {text}"), quiet)))
            .collect();
    }
    let height = columns.iter().map(Vec::len).max().unwrap_or(0);
    (0..height)
        .map(|row| {
            let cell = |column: usize| columns[column].get(row).cloned().unwrap_or_default();
            Line::from(span(
                format!("  {:<24}{:<35}{}", cell(0), cell(1), cell(2)),
                quiet,
            ))
        })
        .collect()
}
```

If `model.agents` is not `Option<AgentsSheet>` with an `agents: Vec<_>` field, count background agents from whichever field `views/agents.rs` reads, and keep the " · ← N agent(s)" format.

`app.rs::compose`: replace `rows.push(chrome::status(chrome_model));` with `rows.extend(chrome::footer(chrome_model));`, and in `reserved` replace the final `+ 1` with `+ chrome::footer(chrome_model).len()`. Leave the other `chrome::status` call in `command_panel_frame` as it is.

`app.rs::handle_key`: at the top of the key path (after the `KeyEventKind::Release` early return), add:

```rust
    // `?` in an empty conversation composer opens Claude Code's shortcuts
    // panel; any other key closes it and is then handled normally.
    if model.shortcuts_open {
        model.shortcuts_open = false;
        if key.code == KeyCode::Char('?') && (key.modifiers - KeyModifiers::SHIFT).is_empty() {
            return Flow::Continue;
        }
    } else if key.code == KeyCode::Char('?')
        && (key.modifiers - KeyModifiers::SHIFT).is_empty()
        && model.composer.is_empty()
        && model.screen == Screen::Agent
        && model.overlay.is_none()
    {
        model.shortcuts_open = true;
        return Flow::Continue;
    }
```

- [ ] **Step 4: Run the crate tests**

Run: `rtk cargo test -p davinci-tui --offline`
Expected: new tests pass. Update older footer tests per the table (for example tests expecting `← for agents` or `! Always Approve`). Keep any test that checks the always-approve warning is never truncated to a harmless word: at narrow widths shorten to `⏵⏵ no prompts`, never to the mode name alone.

- [ ] **Step 5: Commit**

```powershell
git add crates/davinci-tui
git commit -m "feat(tui): Claude Code footer and shortcuts panel"
```

### Task A6: Shell command echo

**Files:**
- Modify: `crates/davinci-tui/src/davinci/model.rs` (`Entry::Shell { command: String, output: Vec<String>, failed: bool }`)
- Modify: `crates/davinci-tui/src/davinci/views/transcript.rs` (render it)
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs` (`run_user_bash::say_output`)
- Test: transcript.rs tests; davinci_interactive.rs tests

**Interfaces:**
- Produces: `Entry::Shell { command, output, failed }` and `fn shell_lines(theme: &Theme, command: &str, output: &[String], width: u16) -> Vec<Line<'static>>`.

- [ ] **Step 1: Write the failing tests** (transcript.rs tests)

```rust
    #[test]
    fn a_shell_command_echoes_like_claude_code() {
        let m = model(120);
        let cc = m.theme.cc();
        let rows = lines(
            &m,
            &[Entry::Shell {
                command: "git log --oneline -3".into(),
                output: vec!["5c14352 init".into()],
                failed: false,
            }],
            120,
        );
        assert_eq!(rows[0].to_string().trim_end(), "! git log --oneline -3");
        assert_eq!(rows[0].spans[0].style.fg, Some(cc.bash));
        assert_eq!(rows[0].spans[0].style.bg, Some(cc.bash_bg));
        assert_eq!(rows[1].to_string(), "  ⎿ \u{a0}5c14352 init");
    }

    #[test]
    fn a_silent_shell_command_says_no_output() {
        let m = model(120);
        let rows = lines(
            &m,
            &[Entry::Shell { command: "exit 3".into(), output: vec![], failed: true }],
            120,
        );
        assert_eq!(rows[1].to_string(), "  ⎿ \u{a0}(No output)");
    }
```

- [ ] **Step 2: Run them.** Run: `rtk cargo test -p davinci-tui --offline a_shell_command_echoes a_silent_shell`. Expected: FAIL (no variant).

- [ ] **Step 3: Implement.**

`model.rs`, in `enum Entry`:

```rust
    /// A `!command` the user ran: the echo and its output (Claude Code's
    /// shell-mode echo, docs/ui/claude-code-reference/shell/51-bang-run2).
    Shell {
        command: String,
        output: Vec<String>,
        failed: bool,
    },
```

`transcript.rs`, in `entry_lines`:

```rust
        Entry::Shell { command, output, .. } => shell_lines(th, command, output, width),
```

and:

```rust
/// Rows under a result: `  ⎿  ` then the first row, five spaces before the rest.
pub const ELBOW: &str = "  ⎿ \u{a0}";

fn shell_lines(theme: &Theme, command: &str, output: &[String], width: u16) -> Vec<Line<'static>> {
    let cc = theme.cc();
    let echo = format!("{command} ");
    let mut rows = vec![Line::from(truncate_run(
        vec![
            span("! ", cc.bash).bg(cc.bash_bg),
            span(echo, cc.text).bg(cc.bash_bg),
        ],
        width,
    ))];
    if output.is_empty() {
        rows.push(Line::from(span(format!("{ELBOW}(No output)"), cc.inactive)));
        return rows;
    }
    for (index, row) in output.iter().enumerate() {
        let lead = if index == 0 { ELBOW } else { "     " };
        rows.push(Line::from(truncate_run(
            vec![span(lead, cc.inactive), span(row.clone(), theme.text)],
            width,
        )));
    }
    rows
}
```

`.bg()` on a `Span` needs `use ratatui::style::Stylize;` (ui.rs already imports it; transcript.rs does not). This part also adds, and Parts B1, B3 and B5 rely on, these imports at the top of `transcript.rs`: `use ratatui::style::{Color, Modifier, Style, Stylize}; use ratatui::text::Span;` and `run_width` in the `crate::davinci::ui::{…}` list (today it is imported only inside the tests module). Under `NO_COLOR` `cc.bash_bg` is `Reset`, so no background is drawn.

`davinci_interactive.rs::run_user_bash`: replace the body of the `say_output` closure after `shell.model.transcript.push(Entry::Gap);` with:

```rust
        let lines: Vec<String> = output
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .take(20)
            .map(|line| clip(line, 100))
            .collect();
        shell.model.transcript.push(Entry::Shell {
            command: command.to_string(),
            output: lines,
            failed,
        });
```

No existing davinci_interactive test covers `!command` output (the `user_bash` tests in `extension_host.rs` cover the extension event only), so nothing else there changes.

- [ ] **Step 4: Run both crates.**

Run: `rtk cargo test -p davinci-tui --offline` and `rtk cargo test -p davinci-coding-agent --offline`
Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add crates/davinci-tui crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "feat(tui): echo shell-mode commands like Claude Code"
```

### Checkpoint A: deliver Part A

- [ ] Run `rtk cargo fmt --all -- --check`, `rtk cargo clippy -p davinci-tui -p davinci-coding-agent --all-targets --offline -- -D warnings`, `rtk cargo test -p davinci-tui --offline`, `rtk cargo test -p davinci-coding-agent --offline`.
- [ ] Capture davinci with `scripts/ui/capture_tui.py --phase ui --bin <target\release\davinci.exe>` and compare these text rows with the reference: `03-slash-all` rows above the rule, `05-slash-comp`, `09-bang` (composer), `14`–`18` footers (mode names differ only by davinci mode names), `26-enter-on-comp` (the transcript must show `❯ /compact`, not a composer holding `/compact`).
- [ ] Follow "Deliver changes to the executable the user actually runs" in CLAUDE.md: release build, back up `%USERPROFILE%\.cargo\bin\davinci.exe`, replace it, compare SHA-256, smoke-check `davinci --version`.
- [ ] Push `ui-cc-a-input`, open the PR, stop and show the user.

---

# Part B: Transcript (branch `ui-cc-b-transcript`)

### Task B1: User message echo

**Files:** `crates/davinci-tui/src/davinci/views/transcript.rs` (`Entry::User` arm). Test in the same file.

- [ ] **Step 1: Failing test**

```rust
    #[test]
    fn a_user_turn_sits_on_the_reference_background() {
        let m = model(120);
        let cc = m.theme.cc();
        let rows = lines(&m, &[Entry::user("run `echo hello` now")], 120);
        assert_eq!(rows[0].to_string(), "❯ run echo hello now ");
        assert_eq!(rows[0].spans[0].style.fg, Some(cc.subtle));
        for span in &rows[0].spans {
            assert_eq!(span.style.bg, Some(cc.user_bg), "{:?}", span.content);
        }
        let code = rows[0].spans.iter().find(|s| s.content.contains("echo hello")).unwrap();
        assert_eq!(code.style.fg, Some(cc.permission));
    }
```

- [ ] **Step 2: Run it.** `rtk cargo test -p davinci-tui --offline a_user_turn_sits`. Expected: FAIL.

- [ ] **Step 3: Implement.** Replace the `Entry::User(text)` arm:

```rust
        Entry::User(text) => user_lines(th, text, width),
```

```rust
/// `❯ text` on `user_bg`, one cell past the text on each row, inline code in
/// `permission` without its backticks (turn/34-final rows 7 to 9).
fn user_lines(theme: &Theme, text: &str, width: u16) -> Vec<Line<'static>> {
    let cc = theme.cc();
    let plain = text.replace('`', "");
    let code: Vec<(usize, usize)> = code_ranges(text);
    let wrapped = crate::wrap_text_with_ansi(&plain, width.saturating_sub(3).max(1) as usize);
    let mut offset = 0usize;
    wrapped
        .into_iter()
        .enumerate()
        .map(|(row, content)| {
            let lead = if row == 0 { "❯ " } else { "  " };
            let mut spans = vec![Span::styled(
                lead,
                Style::default().fg(if row == 0 { cc.subtle } else { cc.text }).bg(cc.user_bg),
            )];
            for (index, ch) in content.chars().enumerate() {
                let at = offset + index;
                let fg = if code.iter().any(|(s, e)| at >= *s && at < *e) { cc.permission } else { cc.text };
                spans.push(Span::styled(ch.to_string(), Style::default().fg(fg).bg(cc.user_bg)));
            }
            offset += content.chars().count() + 1;
            spans.push(Span::styled(" ", Style::default().bg(cc.user_bg)));
            Line::from(truncate_run(merge_same_style(spans), width))
        })
        .collect()
}

/// Character ranges (in the backtick-free text) that were inside backticks.
fn code_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut plain_index = 0;
    let mut open: Option<usize> = None;
    for ch in text.chars() {
        if ch == '`' {
            match open.take() {
                Some(start) => ranges.push((start, plain_index)),
                None => open = Some(plain_index),
            }
            continue;
        }
        plain_index += 1;
    }
    ranges
}

/// Join neighbouring spans that share a style, so tests and terminals see
/// runs rather than one span per character.
fn merge_same_style(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for span in spans {
        match out.last_mut() {
            Some(last) if last.style == span.style => {
                let joined = format!("{}{}", last.content, span.content);
                last.content = joined.into();
            }
            _ => out.push(span),
        }
    }
    out
}
```

`offset` counts characters; `wrap_text_with_ansi` drops the space it breaks at, which the `+ 1` accounts for. If wrapping ever collapses runs of spaces, the code coloring on later rows may shift by those spaces; that is acceptable and must never panic. Import `ratatui::style::Style` and `ratatui::text::Span`.

- [ ] **Step 4: Run.** `rtk cargo test -p davinci-tui --offline`. Update `a_user_turn_is_a_shaded_echo_with_no_timestamp` and other tests that expect `❯ ` in `th.text`.
- [ ] **Step 5: Commit** `git commit -am "feat(tui): Claude Code user message echo"`

### Task B2: Markdown rules

**Files:** `crates/davinci-tui/src/davinci/views/markdown.rs`. Tests in the same file.

- [ ] **Step 1: Failing tests**

```rust
    fn cc_text(rows: &[Line<'_>]) -> Vec<String> {
        rows.iter().map(|r| r.to_string()).collect()
    }

    #[test]
    fn markdown_follows_the_claude_code_reference() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let cc = theme.cc();
        let rows = lines(
            &theme,
            "## Summary\n\n- `echo hello` printed **hello**\n- ~~old~~ new\n\n```rust\nlet greeting = \"hello\";\nprintln!(\"{greeting}\");\n```",
            100,
        );
        let text = cc_text(&rows);
        assert_eq!(text[0], "Summary");
        assert!(rows[0].spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(text[1], "", "no rule under a heading");
        assert_eq!(text[2], "- echo hello printed hello");
        let code = rows[2].spans.iter().find(|s| s.content.contains("echo hello")).unwrap();
        assert_eq!(code.style.fg, Some(cc.permission));
        assert_eq!(text[3], "- old new");
        assert_eq!(text[5], "let greeting = \"hello\";", "no fence, no language row, no rule");
        let keyword = rows[5].spans.iter().find(|s| s.content == "let").unwrap();
        assert_eq!(keyword.style.fg, Some(cc.code_keyword));
        let string = rows[5].spans.iter().find(|s| s.content.contains("\"hello\"")).unwrap();
        assert_eq!(string.style.fg, Some(cc.code_string));
    }
```

- [ ] **Step 2: Run.** `rtk cargo test -p davinci-tui --offline markdown_follows`. Expected: FAIL.

- [ ] **Step 3: Implement.**
  1. `Face::span`: code and link color `ctx.theme.cc().permission` instead of `theme.secondary`.
  2. `render_block`, `Block::Heading`: remove the `hair_rule` push (headings are bold only).
  3. `list_rows`: bullet marker `span("- ", ctx.base)` instead of `glyph::TICK` in `border`; ordered labels in `ctx.base` instead of `theme.muted`.
  4. `code_rows`: drop the language row and the `│ ` prefix; no indent (the transcript adds its two columns); color through a Claude Code token map:

```rust
/// Claude Code colors code with the terminal's named colors: keywords blue,
/// strings red, macros (`name!`) cyan, comments inactive, the rest default.
fn code_rows(lang: Option<&str>, text: &str, width: u16, ctx: &Ctx) -> Vec<Line<'static>> {
    use super::highlight::Token;
    let cc = ctx.theme.cc();
    let language = lang.and_then(super::highlight::language_of);
    let body = text.strip_suffix('\n').unwrap_or(text);
    if body.is_empty() {
        return Vec::new();
    }
    body.split('\n')
        .map(|line| {
            let content = clip_ellipsis(&line.trim_end_matches('\r').replace('\t', "    "), width);
            let mut spans = Vec::new();
            for (token, run) in super::highlight::tokens(language, &content) {
                match token {
                    Token::Keyword => spans.push(span(run, cc.code_keyword)),
                    Token::String => spans.push(span(run, cc.code_string)),
                    Token::Comment => spans.push(span(run, cc.inactive)),
                    Token::Number | Token::Plain => {
                        spans.extend(split_macros(&run, ctx.base, cc.code_macro))
                    }
                }
            }
            Line::from(spans)
        })
        .collect()
}

/// The tokenizer treats `!` as part of an identifier and merges plain runs,
/// so `println!(` arrives as one plain run. Color each `name!` as a macro.
fn split_macros(run: &str, base: Color, macro_color: Color) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut rest = run;
    while let Some(bang) = rest.find('!') {
        let before = &rest[..bang];
        let start = before
            .char_indices()
            .rev()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
            .map_or(0, |(i, c)| i + c.len_utf8());
        if start == bang {
            // A lone `!` (`!=`, `!flag`) is an operator, not a macro.
            out.push(span(rest[..=bang].to_string(), base));
        } else {
            if start > 0 {
                out.push(span(before[..start].to_string(), base));
            }
            out.push(span(rest[start..=bang].to_string(), macro_color));
        }
        rest = &rest[bang + 1..];
    }
    if !rest.is_empty() {
        out.push(span(rest.to_string(), base));
    }
    out
}
```

  `highlight::tokens(lang, line) -> Vec<(Token, String)>` is already `pub` (highlight.rs, around line 997). Delete the now unused `CODE_INSET` constant.
  5. Update the module doc comment: replace "a `│` left rule for code, the `·` tick for bullets, a hair rule under the two top heading levels" with the Claude Code rules.

- [ ] **Step 4: Run.** `rtk cargo test -p davinci-tui --offline markdown`. Update `heading_drops_the_hash_and_level_one_gets_a_rule`, `bullet_list_marks_items_and_hangs_continuations`, `fenced_code_names_its_language_and_clips_long_rows`, `inline_code_is_verdigris` to the new rules (keep their clipping, wrapping and no-panic checks).
- [ ] **Step 5: Commit** `git commit -am "feat(tui): Claude Code markdown in replies"`

### Task B3: Tool rows and result elbow

**Files:** `crates/davinci-tui/src/davinci/ui.rs` (`tool_line`), `views/transcript.rs` (`tool_result`, `Entry::Tool` arm).

- [ ] **Step 1: Failing tests** (transcript.rs)

```rust
    #[test]
    fn a_finished_call_has_a_green_bullet_and_a_bold_count() {
        let m = model(120);
        let cc = m.theme.cc();
        let rows = lines(
            &m,
            &[Entry::Tool {
                state: State::Delta,
                instrument: "officina".into(),
                target: "edit README.md".into(),
                duration: Some("0.1s".into()),
                summary: Some("Added 2 lines, removed 1 line".into()),
                output: vec![],
            }],
            120,
        );
        assert_eq!(rows[0].to_string().trim_end(), "● Update(README.md)");
        assert_eq!(rows[0].spans[0].style.fg, Some(cc.success));
        let args = rows[0].spans.iter().find(|s| s.content.contains("README")).unwrap();
        assert_eq!(args.style.fg, Some(m.theme.text));
        assert_eq!(rows[1].to_string(), "  ⎿ \u{a0}Added 2 lines, removed 1 line");
        let two = rows[1].spans.iter().find(|s| s.content == "2").unwrap();
        assert!(two.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_running_call_blinks_an_inactive_bullet() {
        let mut m = model(120);
        let cc = m.theme.cc();
        let entry = Entry::Tool {
            state: State::Done,
            instrument: "officina".into(),
            target: "write notes.txt".into(),
            duration: None,
            summary: None,
            output: vec![],
        };
        m.running = true;
        m.tick = 0;
        let on = lines(&m, std::slice::from_ref(&entry), 120);
        assert!(on[0].to_string().starts_with("● Write(notes.txt)"));
        assert_eq!(on[0].spans[0].style.fg, Some(cc.inactive));
        m.tick = 1;
        let off = lines(&m, std::slice::from_ref(&entry), 120);
        assert!(off[0].to_string().starts_with("  Write(notes.txt)"));
    }
```

The live instrument for read, edit and write calls is `"instrumenta"` (`instrument_of`, davinci_interactive.rs around line 223); captions and grouping key on the target verb, so any instrument other than `manus` works in these tests.

- [ ] **Step 2: Run.** Expected: FAIL.

- [ ] **Step 3: Implement.**
  1. `ui::tool_line` gains `tick: u64` and `live: bool` parameters (update all callers; pass `model.tick` and `model.running`). `live` matters: some finished rows never get a duration (the cogitator rows around davinci_interactive.rs:2005, the `/jobs kill` result around 8688), and they must not blink after the turn. Remove the `summary` parameter (the result row now carries the outcome; an unused parameter fails clippy). Bullet:

```rust
    let cc = theme.cc();
    let failed = matches!(state, State::Failed | State::Attention);
    let running = live
        && duration.is_none()
        && !failed
        && !matches!(state, State::Skipped | State::Queued);
    // Under NO_COLOR the state glyph (✓, Δ, ×, …) still carries the meaning.
    let mark = if theme.no_color {
        state.glyph()
    } else if running && tick % 2 == 1 {
        " "
    } else {
        "●"
    };
    let color = if failed {
        cc.error
    } else if running {
        cc.inactive
    } else {
        cc.success
    };
```

  Label bold in `theme.text`; `(`, argument and `)` in `theme.text`. Drop the ` · duration` / ` · summary` tail (the result row carries the outcome).
  2. `tool_result`: `ELBOW` in `cc.inactive`, then the text in `theme.text` with every run of ASCII digits split into its own bold span:

```rust
fn tool_result(theme: &Theme, text: &str, width: u16) -> Line<'static> {
    let cc = theme.cc();
    let mut spans = vec![span(ELBOW, cc.inactive)];
    spans.extend(bold_numbers(&clip_ellipsis(text, width.saturating_sub(5)), theme.text));
    Line::from(truncate_run(spans, width))
}

/// `Added 2 lines` with `2` bold, as Claude Code draws counts.
fn bold_numbers(text: &str, color: Color) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut digits = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() != digits && !current.is_empty() {
            let mut piece = span(std::mem::take(&mut current), color);
            if digits {
                piece.style = piece.style.add_modifier(Modifier::BOLD);
            }
            out.push(piece);
        }
        digits = ch.is_ascii_digit();
        current.push(ch);
    }
    if !current.is_empty() {
        let mut piece = span(current, color);
        if digits {
            piece.style = piece.style.add_modifier(Modifier::BOLD);
        }
        out.push(piece);
    }
    out
}
```

  3. Summaries: in `davinci_interactive.rs`, where a tool entry's `summary` is set (search `summarised(`), produce the Claude Code phrases: read `Read N lines`, edit `Added N lines, removed M lines` (omit a zero part; singular `line` for 1), write `Wrote N lines to <file name>`, shell `N lines` of output stays as is. Add a unit test for the phrase builder (`fn change_summary(adds: u32, dels: u32) -> String`):

```rust
    #[test]
    fn change_summary_uses_claude_code_phrases() {
        assert_eq!(change_summary(1, 0), "Added 1 line");
        assert_eq!(change_summary(2, 1), "Added 2 lines, removed 1 line");
        assert_eq!(change_summary(0, 3), "Removed 3 lines");
    }
```

  ```rust
  pub fn change_summary(adds: u32, dels: u32) -> String {
      let lines = |n: u32| if n == 1 { "line" } else { "lines" };
      match (adds, dels) {
          (0, 0) => "No changes".into(),
          (a, 0) => format!("Added {a} {}", lines(a)),
          (0, d) => format!("Removed {d} {}", lines(d)),
          (a, d) => format!("Added {a} {}, removed {d} {}", lines(a), lines(d)),
      }
  }
  ```

- [ ] **Step 4: Run** both crates' tests; update `a_tool_call_is_one_line_with_no_box` and `collapsed_success_shows_only_the_first_output_row` expectations (`● Shell(cargo fmt)` stays; the ` · 0.1s` tail goes).
- [ ] **Step 5: Commit** `git commit -am "feat(tui): Claude Code tool rows and result elbow"`

### Task B4: Group read, search, list and shell calls

**Files:** `views/transcript.rs` (`lines`, `tail_lines`). Test in the same file.

**Interfaces:**
- Produces: `enum Explore { Read, Search, List, Shell }`, `fn explore_kind(instrument: &str, target: &str) -> Option<Explore>`, `fn group_rows(model: &Model, calls: &[(Explore, &str, bool)], width: u16) -> Vec<Line<'static>>` (kind, target, finished).

- [ ] **Step 1: Failing tests**

```rust
    fn call(instrument: &str, target: &str, done: bool) -> Entry {
        Entry::Tool {
            state: State::Done,
            instrument: instrument.into(),
            target: target.into(),
            duration: done.then(|| "0.1s".to_string()),
            summary: None,
            output: vec![],
        }
    }

    #[test]
    fn finished_exploration_folds_into_one_summary_row() {
        let m = model(120);
        let rows = lines(
            &m,
            &[call("lector", "read README.md", true), call("manus", "echo hello", true)],
            120,
        );
        let text: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
        assert_eq!(text, vec!["  Read 1 file, ran 1 shell command".to_string()]);
        let one = rows[0].spans.iter().find(|s| s.content == "1").unwrap();
        assert!(one.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn running_exploration_names_the_newest_call() {
        let mut m = model(120);
        m.running = true;
        let rows = lines(
            &m,
            &[call("lector", "read README.md", true), call("manus", "echo hello", false)],
            120,
        );
        assert_eq!(rows[0].to_string().trim_end(), "● Reading 1 file, running 1 shell command…");
        assert_eq!(rows[1].to_string(), "  ⎿ \u{a0}$ echo hello");
    }

    #[test]
    fn verbose_view_and_failures_keep_every_call() {
        let mut m = model(120);
        m.show_tool_output = true;
        let calls = [call("lector", "read a.rs", true), call("lector", "read b.rs", true)];
        assert!(lines(&m, &calls, 120).len() >= 2);
        m.show_tool_output = false;
        let mut failed = call("manus", "exit 3", true);
        if let Entry::Tool { state, .. } = &mut failed {
            *state = State::Failed;
        }
        let rows = lines(&m, &[call("lector", "read a.rs", true), failed], 120);
        assert!(rows.iter().any(|r| r.to_string().contains("Shell(exit 3)")));
    }
```

Use the instrument names the existing code uses for read calls (see `tool_state`/`tool_caption`: read targets start with `read `; shell calls use instrument `manus`).

- [ ] **Step 2: Run.** Expected: FAIL.

- [ ] **Step 3: Implement.** In `lines` and `tail_lines`, walk entries and gather runs of consecutive `Entry::Tool` (skipping `Entry::Gap` between them) whose `explore_kind` is `Some` and whose state is not `Failed`. A run of one or more such calls renders through `group_rows` when `!model.show_tool_output`; otherwise entries render as today.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Explore {
    Read,
    Search,
    List,
    Shell,
}

fn explore_kind(instrument: &str, target: &str) -> Option<Explore> {
    if instrument == "manus" {
        return Some(Explore::Shell);
    }
    match target.split_once(' ').map_or(target, |(verb, _)| verb) {
        "read" => Some(Explore::Read),
        "search" | "grep" | "find" => Some(Explore::Search),
        "list" | "ls" => Some(Explore::List),
        _ => None,
    }
}

/// `Read 1 file, ran 1 shell command` (done) or `Reading 1 file, running 1
/// shell command…` plus the newest call (running), numbers bold.
fn group_rows(model: &Model, calls: &[(Explore, &str, bool)], width: u16) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    // A call without a duration is only in flight while the turn runs.
    let running = model.running && calls.iter().any(|(_, _, done)| !done);
    let mut order: Vec<Explore> = Vec::new();
    for (kind, _, _) in calls {
        if !order.contains(kind) {
            order.push(*kind);
        }
    }
    let clause = |kind: Explore, n: usize| -> String {
        let s = |one: &str, many: &str| if n == 1 { one.to_string() } else { many.to_string() };
        match (kind, running) {
            (Explore::Read, false) => format!("read {n} {}", s("file", "files")),
            (Explore::Read, true) => format!("reading {n} {}", s("file", "files")),
            (Explore::Search, false) => format!("searched for {n} {}", s("pattern", "patterns")),
            (Explore::Search, true) => format!("searching for {n} {}", s("pattern", "patterns")),
            (Explore::List, false) => format!("listed {n} {}", s("directory", "directories")),
            (Explore::List, true) => format!("listing {n} {}", s("directory", "directories")),
            (Explore::Shell, false) => format!("ran {n} shell {}", s("command", "commands")),
            (Explore::Shell, true) => format!("running {n} shell {}", s("command", "commands")),
        }
    };
    let mut sentence = order
        .iter()
        .map(|kind| clause(*kind, calls.iter().filter(|(k, _, _)| k == kind).count()))
        .collect::<Vec<_>>()
        .join(", ");
    if let Some(first) = sentence.get(0..1) {
        sentence = format!("{}{}", first.to_uppercase(), &sentence[1..]);
    }
    if !running {
        let mut spans = vec![span("  ", cc.inactive)];
        spans.extend(bold_numbers(&sentence, cc.inactive));
        return vec![Line::from(truncate_run(spans, width))];
    }
    let bullet = if model.tick % 2 == 1 { "  " } else { "● " };
    let mut spans = vec![span(bullet, cc.inactive)];
    spans.extend(bold_numbers(&format!("{sentence}…"), th.text));
    let (kind, target, _) = calls.last().expect("a group has a call");
    let newest = match kind {
        Explore::Shell => format!("$ {target}"),
        _ => target.split_once(' ').map_or(*target, |(_, rest)| rest).to_string(),
    };
    vec![
        Line::from(truncate_run(spans, width)),
        Line::from(truncate_run(
            vec![span(format!("{ELBOW}{}", clip_ellipsis(&newest, width.saturating_sub(5))), cc.inactive)],
            width,
        )),
    ]
}
```

`bold_numbers` is the helper from Task B3 (move it to `transcript.rs` or `ui.rs` so both use one copy). `tail_lines` must group the same way: build the grouped block list first (a `Vec<Vec<Line>>`, one per rendered block), then take rows from the end, so `tail_lines_matches_a_full_render_tailed_at_every_height` keeps passing.

- [ ] **Step 4: Run** `rtk cargo test -p davinci-tui --offline transcript`. Expected: PASS, including `tail_lines_matches_a_full_render_tailed_at_every_height`. Three existing tests render a lone finished `manus` call and will now see `  Ran 1 shell command`: `a_tool_call_is_one_line_with_no_box`, `collapsed_success_shows_only_the_first_output_row` and `tool_glyphs_survive_no_color`. Change their entries to a call that is not grouped (for example target `fetch https://example.com`, caption `Fetch`), so they keep testing the single-call row with their current assertions; `collapsed_success_shows_only_the_first_output_row` must stay collapsed, so do not switch it to the verbose view.
- [ ] **Step 5: Commit** `git commit -am "feat(tui): fold exploration calls into one row"`

### Task B5: Numbered diff rows

**Files:** `crates/davinci-tui/src/davinci/model.rs` (`Hunk.line`), `views/transcript.rs` (`Entry::Delta` arm, `hunk_line`), `crates/davinci-coding-agent/src/davinci_interactive.rs` (`hunks_from_diff`).

- [ ] **Step 1: Failing tests**

transcript.rs:

```rust
    #[test]
    fn a_diff_has_line_numbers_and_full_width_backgrounds() {
        let m = model(60);
        let cc = m.theme.cc();
        let rows = lines(
            &m,
            &[Entry::Delta {
                path: "README.md".into(),
                adds: 1,
                dels: 1,
                hunks: vec![
                    Hunk::at(HunkKind::Context, 3, "A tiny project."),
                    Hunk::at(HunkKind::Del, 4, "Edited."),
                    Hunk::at(HunkKind::Add, 4, "Changed."),
                ],
            }],
            60,
        );
        assert_eq!(rows[0].to_string(), "  ⎿ \u{a0}Added 1 line, removed 1 line");
        assert_eq!(rows[1].to_string().trim_end(), "      3  A tiny project.");
        assert_eq!(rows[2].to_string().trim_end(), "      4 -Edited.");
        assert_eq!(rows[3].to_string().trim_end(), "      4 +Changed.");
        assert_eq!(rows[3].to_string().chars().count(), 60, "background reaches the edge");
        let sign = rows[3].spans.iter().find(|s| s.content.contains('+')).unwrap();
        assert_eq!(sign.style.fg, Some(cc.diff_add));
        assert_eq!(sign.style.bg, Some(cc.diff_add_bg));
        let number = rows[1].spans.iter().find(|s| s.content.contains('3')).unwrap();
        assert!(number.style.add_modifier.contains(Modifier::DIM));
    }
```

davinci_interactive.rs:

```rust
    #[test]
    fn hunks_from_diff_keeps_line_numbers() {
        let (_, _, hunks) = hunks_from_diff("+12 added\n- 8 removed\n  4 kept\n    ...\n");
        assert_eq!(hunks[0].line, Some(12));
        assert_eq!(hunks[1].line, Some(8));
        assert_eq!(hunks[2].line, Some(4));
        assert_eq!(hunks[3].line, None);
    }
```

- [ ] **Step 2: Run.** Expected: FAIL (`no function at`, `no field line`).

- [ ] **Step 3: Implement.**

`model.rs`:

```rust
pub struct Hunk {
    pub kind: HunkKind,
    pub text: String,
    /// The file line this row shows, when the diff said.
    pub line: Option<u32>,
}

impl Hunk {
    pub fn new(kind: HunkKind, text: &str) -> Self {
        Self { kind, text: text.to_string(), line: None }
    }

    pub fn at(kind: HunkKind, line: u32, text: &str) -> Self {
        Self { kind, text: text.to_string(), line: Some(line) }
    }
}
```

`hunks_from_diff`: parse the digits already counted (`trimmed[..digits].parse::<u32>().ok()`) and build `Hunk::at` when present, `Hunk::new` otherwise.

`transcript.rs`, `Entry::Delta` arm: first row `tool_result(th, &change_summary(*adds, *dels), width)` (import `change_summary` from wherever Task B3 put it; if it lives in the coding-agent crate, add a copy named the same in `transcript.rs` and have the coding agent call the TUI one), then the hunks (same caps as today), then the `… N more rows` row as `      …` in `cc.inactive`. Replace `hunk_line`:

```rust
/// `      N sign text` (docs/ui/claude-code-reference/turn2/44-final rows 8 to
/// 13): five spaces, the number right-aligned, the sign, the text; changed
/// rows carry their background to the right edge.
fn hunk_line(
    theme: &Theme,
    language: Option<super::highlight::Lang>,
    hunk: &crate::davinci::model::Hunk,
    number_width: usize,
    width: u16,
) -> Line<'static> {
    let cc = theme.cc();
    let number = hunk.line.map_or(String::new(), |n| n.to_string());
    let gutter = format!(" {number:>number_width$} ");
    let (sign, sign_color, bg) = match hunk.kind {
        HunkKind::Add => ("+", cc.diff_add, Some(cc.diff_add_bg)),
        HunkKind::Del => ("-", cc.diff_del, Some(cc.diff_del_bg)),
        HunkKind::Context => (" ", cc.diff_text, None),
    };
    let style = |fg: Color| {
        let s = Style::default().fg(fg);
        match bg {
            Some(bg) => s.bg(bg),
            None => s,
        }
    };
    let mut gutter_style = style(if bg.is_some() { sign_color } else { cc.diff_text });
    if bg.is_none() {
        gutter_style = gutter_style.add_modifier(Modifier::DIM);
    }
    let room = width.saturating_sub(5 + gutter.chars().count() as u16 + 1);
    let text = clip_ellipsis(&hunk.text, room);
    let mut spans = vec![
        Span::raw("     "),
        Span::styled(gutter, gutter_style),
        Span::styled(sign.to_string(), style(sign_color)),
    ];
    for piece in super::highlight::spans(theme, language, &text, cc.diff_text) {
        let fg = piece.style.fg.unwrap_or(cc.diff_text);
        spans.push(Span::styled(piece.content.to_string(), style(fg)));
    }
    let used = run_width(&spans);
    if let Some(bg) = bg {
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used) as usize),
            Style::default().bg(bg),
        ));
    }
    Line::from(spans)
}
```

`number_width` is the widest `line` in the block (`hunks.iter().filter_map(|h| h.line).max().map_or(1, |n| n.to_string().len())`).

- [ ] **Step 4: Run** both crates. Update `a_delta_block_names_its_path_and_its_counts`, `hunks_sit_behind_a_single_left_rule_with_no_line_numbers`, `a_live_delta_caps_at_eight_collapsed_and_forty_expanded`, `a_delta_hunk_colours_keywords_and_the_add_sign` to the new layout (keep their caps).
- [ ] **Step 5: Commit** `git commit -am "feat(tui): numbered Claude Code diffs"`

### Task B6: Working line and completion line

**Files:** `crates/davinci-tui/src/davinci/model.rs` (`Working`, `Entry::Done`), `views/opera.rs`, `views/transcript.rs`, `crates/davinci-coding-agent/src/davinci_interactive.rs` (turn start around line 1435 and turn end around line 1710).

**Interfaces:**
- Produces: `Working { seconds, tokens, thinking, interrupting, verb_seed: u64, thought_for: Option<u64>, reasoning: bool }`, `Working::past_verb(&self) -> &'static str`, `Entry::Done { verb: String, seconds: u64 }`, `pub const SPINNER_FRAMES: [&str; 6]`.

- [ ] **Step 1: Failing tests** (opera.rs; replace the old tests in this module with these, keeping `nothing_is_drawn_between_turns` and `it_never_overruns_the_window`)

```rust
    fn working() -> Working {
        Working {
            seconds: 2,
            tokens: 138,
            thinking: Some("high".into()),
            reasoning: true,
            ..Working::default()
        }
    }

    #[test]
    fn the_working_line_matches_the_reference_text() {
        let mut m = model(120);
        m.working = Some(Working { verb_seed: 1, ..working() });
        m.tick = 1;
        assert_eq!(
            text(&lines(&m)[0]),
            "✢ Levitating… (2s · ↓ 138 tokens · thinking with high effort)"
        );
    }

    #[test]
    fn the_glyph_bounces_through_six_frames() {
        let mut m = model(120);
        m.working = Some(working());
        let seen: Vec<char> = (0..10)
            .map(|tick| {
                m.tick = tick;
                text(&lines(&m)[0]).chars().next().unwrap()
            })
            .collect();
        assert_eq!(seen, vec!['·', '✢', '*', '✶', '✻', '✽', '✻', '✶', '*', '✢']);
    }

    #[test]
    fn the_verb_holds_for_the_turn_and_shimmers() {
        let mut m = model(120);
        m.working = Some(Working { verb_seed: 1, seconds: 0, ..working() });
        let first = text(&lines(&m)[0]);
        m.working.as_mut().unwrap().seconds = 9;
        let later = text(&lines(&m)[0]);
        assert_eq!(first.split('…').next(), later.split('…').next());
        m.tick = 4; // the window lights letters 1 to 3 on tick 4
        let cc = m.theme.cc();
        let colors: Vec<_> = lines(&m)[0].spans.iter().map(|s| s.style.fg).collect();
        assert!(colors.contains(&Some(cc.claude)));
        assert!(colors.contains(&Some(cc.claude_shimmer)));
    }

    #[test]
    fn after_reasoning_it_says_how_long_it_thought() {
        let mut m = model(120);
        m.working = Some(Working { reasoning: false, thought_for: Some(1), ..working() });
        assert!(text(&lines(&m)[0]).ends_with("(2s · ↓ 138 tokens · thought for 1s)"));
    }

    #[test]
    fn esc_to_interrupt_is_not_on_the_working_line() {
        let mut m = model(120);
        m.working = Some(working());
        assert!(!text(&lines(&m)[0]).contains("esc"));
    }
```

transcript.rs:

```rust
    #[test]
    fn a_finished_turn_ends_with_the_done_line() {
        let m = model(120);
        let rows = lines(&m, &[Entry::Done { verb: "Worked".into(), seconds: 7 }], 120);
        assert_eq!(rows[0].to_string(), "✻ Worked for 7s");
        assert_eq!(rows[0].spans[0].style.fg, Some(m.theme.cc().inactive));
    }
```

- [ ] **Step 2: Run.** Expected: FAIL.

- [ ] **Step 3: Implement.**

`model.rs`, `Working`: add `verb_seed: u64`, `thought_for: Option<u64>`, `reasoning: bool` (all covered by `#[derive(Default)]`). `fixtures.rs` (around line 1852) builds `Working { seconds, tokens, thinking, interrupting }` with every field listed; add `..Default::default()` there. `opera.rs` imports only `Line`; add `use ratatui::style::Color; use ratatui::text::Span;`. Replace `verb()`:

```rust
    pub fn verb(&self) -> &'static str {
        if self.interrupting {
            return "Interrupting";
        }
        const VERBS: [&str; 20] = [
            "Boondoggling", "Levitating", "Envisioning", "Unraveling", "Pondering",
            "Percolating", "Mulling", "Noodling", "Ruminating", "Simmering",
            "Brewing", "Crunching", "Churning", "Conjuring", "Tinkering",
            "Whittling", "Musing", "Deliberating", "Cogitating", "Synthesizing",
        ];
        VERBS[(self.verb_seed as usize) % VERBS.len()]
    }

    /// The past-tense word on the completion line.
    pub fn past_verb(&self) -> &'static str {
        const PAST: [&str; 6] = ["Worked", "Crunched", "Churned", "Baked", "Brewed", "Cooked"];
        PAST[(self.verb_seed as usize) % PAST.len()]
    }
```

`Entry::Done { verb: String, seconds: u64 }` in `enum Entry`, rendered in `transcript.rs`:

```rust
        Entry::Done { verb, seconds } => {
            let cc = th.cc();
            vec![Line::from(vec![
                span("✻ ", cc.inactive),
                span(format!("{verb} for {}", duration_words(*seconds)), cc.inactive),
            ])]
        }
```

```rust
/// `7s`, `1m 5s`, `1h 2m`.
fn duration_words(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}
```

`opera.rs::lines`:

```rust
pub const SPINNER_FRAMES: [&str; 6] = ["·", "✢", "*", "✶", "✻", "✽"];

fn frame(tick: u64, animate: bool) -> &'static str {
    if !animate {
        return "✻";
    }
    let step = (tick % 10) as usize;
    SPINNER_FRAMES[if step < 6 { step } else { 10 - step }]
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let cc = model.theme.cc();
    let Some(working) = &model.working else {
        return Vec::new();
    };
    let mut spans = vec![span(format!("{} ", frame(model.tick, model.animate)), cc.claude)];
    spans.extend(shimmer(working.verb(), model.tick, model.animate, cc.claude, cc.claude_shimmer));
    spans.push(span("… ", cc.claude));
    let mut facts = vec![format!("{}s", working.seconds)];
    if working.tokens > 0 {
        facts.push(format!("↓ {} tokens", thousands(working.tokens)));
    }
    let reasoning = match (&working.thinking, working.reasoning, working.thought_for) {
        (Some(effort), true, _) => Some(format!("thinking with {effort} effort")),
        (_, false, Some(s)) => Some(format!("thought for {s}s")),
        _ => None,
    };
    let lively = working.reasoning;
    if let Some(reasoning) = reasoning {
        facts.push(reasoning);
    }
    while !facts.is_empty() {
        let mut tail = vec![span("(", cc.inactive)];
        for (index, fact) in facts.iter().enumerate() {
            if index > 0 {
                tail.push(span(" · ", cc.inactive));
            }
            let last = index == facts.len() - 1;
            let color = if last && lively && model.tick % 2 == 0 { cc.inactive_shimmer } else { cc.inactive };
            tail.push(span(fact.clone(), color));
        }
        tail.push(span(")", cc.inactive));
        if run_width(&spans) + run_width(&tail) <= model.width.saturating_sub(2) {
            spans.extend(tail);
            break;
        }
        facts.pop();
    }
    vec![Line::from(truncate_run(spans, model.width))]
}

/// The verb with a 3-letter window in the shimmer color, one letter a tick.
fn shimmer(word: &str, tick: u64, animate: bool, base: Color, light: Color) -> Vec<Span<'static>> {
    let chars: Vec<char> = word.chars().collect();
    if !animate || chars.is_empty() {
        return vec![span(word.to_string(), base)];
    }
    let cycle = chars.len() + 3;
    let start = (tick as usize) % cycle;
    let mut out = Vec::new();
    for (index, ch) in chars.iter().enumerate() {
        let lit = index + 3 >= start && index < start;
        let color = if lit { light } else { base };
        match out.last_mut() {
            Some(Span { style, content, .. }) if style.fg == Some(color) => {
                let joined = format!("{content}{ch}");
                *content = joined.into();
            }
            _ => out.push(span(ch.to_string(), color)),
        }
    }
    out
}
```

Check `the_working_line_matches_the_reference_text`: at tick 1, `frame` is `✢`; the text join ignores colors, so the shimmer does not change the text.

`davinci_interactive.rs`:
- Turn start (`model.working = Some(Working { … })`, around line 1435): add `verb_seed: turn_counter`, where `turn_counter` is a `u64` kept on the shell/session state and incremented per turn (if there is no such counter, use the number of `Entry::User` entries in `model.transcript`), `reasoning: false`, `thought_for: None`, `..Working::default()`.
- Where a live `Entry::Thinking` is created or updated, set `working.reasoning = true`. In `turn.settle_thinking(model)` (or right after each call), set `working.reasoning = false` and `working.thought_for = Some(seconds)` from the settled `Entry::Thinking { seconds, .. }`.
- Turn end (`model.working = None;`, around line 1710): read the working state first, then push the done line after `turn.close(model, interrupted)` when not interrupted:

```rust
    let finished = model.working.take();
    turn.close(model, interrupted);
    if let (false, Some(working)) = (interrupted, finished) {
        model.transcript.push(Entry::Gap);
        model.transcript.push(Entry::Done {
            verb: working.past_verb().to_string(),
            seconds: started.elapsed().as_secs(),
        });
    }
```

  (Keep the existing order of `turn.close` and the `turn_outcome` loop; the done line goes before the outcome entries only if Claude Code order requires it: in `turn/34-final` it is the last row of the turn.)

- [ ] **Step 4: Run** both crates. The opera tests replaced in Step 1 cover the old ones.
- [ ] **Step 5: Commit** `git commit -am "feat(tui): Claude Code working and completion lines"`

### Task B7: Hide reasoning in the normal view; banner

**Files:** `views/transcript.rs` (`Entry::Thinking` arm), `views/startup.rs`.

- [ ] **Step 1: Failing tests**

transcript.rs:

```rust
    #[test]
    fn reasoning_is_hidden_unless_verbose() {
        let mut m = model(120);
        let entry = [Entry::thinking("I will read the file.", false, 3)];
        assert!(lines(&m, &entry, 120).is_empty());
        m.show_tool_output = true;
        assert!(!lines(&m, &entry, 120).is_empty());
    }
```

startup.rs:

```rust
    #[test]
    fn the_banner_follows_the_reference_layout() {
        let mut m = model(120);
        m.model_name = "gpt-5.6-luna".into();
        m.thinking_level = "high".into();
        let info = Startup { cwd: "C:\\Users\\me\\project".into(), ..Default::default() };
        let rows = banner(&m, &info);
        let cc = m.theme.cc();
        let first = rows[0].to_string();
        let column = first[..first.find("DaVinci").unwrap()].chars().count();
        assert_eq!(column, 11, "{first}");
        assert_eq!(rows[0].spans[0].style.fg, Some(cc.claude));
        let name = rows[0].spans.iter().find(|s| s.content == "DaVinci").unwrap();
        assert!(name.style.add_modifier.contains(Modifier::BOLD));
        assert!(rows[1].to_string().contains("gpt-5.6-luna with high effort"));
        let tip = rows.last().unwrap().to_string();
        assert!(tip.starts_with("  Switch models anytime with /model"), "{tip}");
    }
```

(Adjust the `Startup` construction to its real fields; set `USERPROFILE`/`HOME` handling aside: the test path does not start with the home directory.)

- [ ] **Step 2: Run.** Expected: FAIL.

- [ ] **Step 3: Implement.**
- `Entry::Thinking` arm: `if !model.show_tool_output { return Vec::new(); }` then the existing `thinking_lines`.
- `startup::banner`:

```rust
pub fn banner(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let mut name = span("DaVinci", th.text);
    name.style = name.style.add_modifier(Modifier::BOLD);
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let cwd = if !home.is_empty() && info.cwd.starts_with(&home) {
        format!("~{}", &info.cwd[home.len()..])
    } else {
        info.cwd.clone()
    };
    let rows = [
        vec![span(" ██████╗   ", cc.claude), name, span(format!(" v{}", env!("CARGO_PKG_VERSION")), cc.inactive)],
        vec![
            span(" ██   ██║  ", cc.claude),
            span(format!("{} with {} effort", model.model_name, model.thinking_level), cc.inactive),
        ],
        vec![span(" ██████╔╝  ", cc.claude), span(clip_ellipsis(&cwd, model.width.saturating_sub(12)), cc.inactive)],
        vec![],
        vec![
            span("  Switch models anytime with ", th.text),
            span("/model", cc.permission),
            span(". Type ", th.text),
            span("?", cc.permission),
            span(" for shortcuts.", th.text),
        ],
    ];
    rows.into_iter().map(|row| Line::from(truncate_run(row, model.width))).collect()
}
```

In `startup.rs` add `use ratatui::style::Modifier;` and `clip_ellipsis` to the `crate::davinci::ui::{…}` import, and remove the now unused `paper_label`. The test needs `use ratatui::style::Modifier;` too.

- [ ] **Step 4: Run** `rtk cargo test -p davinci-tui --offline`. Update startup and `live_reasoning_shows_its_tail_and_collapses_to_one_row_when_done` tests (the verbose view still shows the tail).
- [ ] **Step 5: Commit** `git commit -am "feat(tui): hide reasoning and restyle the welcome banner"`

### Checkpoint B

- [ ] Same commands as Checkpoint A, plus a live check: run `davinci` with a real model on a scratch repository, ask it to read a file, run a shell command and edit a file; compare with `turn/34-final` and `turn2/44-final` side by side. Note every visible difference in the PR.
- [ ] Deliver the installed binary (CLAUDE.md delivery rule), push `ui-cc-b-transcript`, open the PR, stop and show the user.

---

# Part C: Panels (branch `ui-cc-c-panels`)

### Task C1: Permission panel

**Files:** `crates/davinci-tui/src/davinci/model.rs` (`Ask`), `views/ask.rs`, `crates/davinci-coding-agent/src/davinci_interactive.rs` (`permission_ask`).

**Interfaces:**
- Produces: new `Ask` fields `pub subject: String`, `pub question: String`, `pub preview: Vec<Hunk>`, `pub kind: AskKind` with `enum AskKind { #[default] List, Shell, File }`; `fn permission_title(tool: &str, exists: bool) -> String`; `fn approval_preview(request: &ToolApprovalRequest, cwd: &Path) -> (Vec<Hunk>, bool)`.

`Ask` derives `Default`, `PartialEq` and `Eq`, so existing literals add `..Default::default()` where the compiler asks, `AskKind` derives `Debug, Clone, Copy, Default, PartialEq, Eq` with `#[default] List`, and `Hunk` must derive `Default`, `PartialEq` and `Eq` (add what is missing; `HunkKind` needs `Default` with `#[default] Context`).

- [ ] **Step 1: Failing tests** (ask.rs tests)

```rust
    fn approval(kind: AskKind, title: &str, subject: &str, question: &str, preview: Vec<Hunk>) -> Model {
        let mut m = model(120);
        m.overlay = Some(Overlay::Ask);
        m.ask = Ask {
            title: title.into(),
            key: "/permissions".into(),
            subject: subject.into(),
            question: question.into(),
            preview,
            kind,
            items: vec![
                PickerItem::new("Yes", ""),
                PickerItem::new("Yes, and don't ask again for: cargo *", ""),
                PickerItem::new("No", ""),
            ],
            ..Default::default()
        };
        m
    }

    #[test]
    fn a_shell_approval_matches_the_reference() {
        let m = approval(AskKind::Shell, "PowerShell command", "cargo --version", "Do you want to proceed?", vec![]);
        let cc = m.theme.cc();
        let rows = lines(&m);
        let text: Vec<String> = rows.iter().map(|r| r.to_string().trim_end().to_string()).collect();
        assert!(text[0].chars().all(|c| c == '─') && text[0].chars().count() == 120);
        assert_eq!(rows[0].spans[0].style.fg, Some(cc.permission));
        assert_eq!(text[1], " PowerShell command");
        assert_eq!(text[2], "");
        assert_eq!(text[3], "   cargo --version");
        assert_eq!(text[4], "");
        assert_eq!(text[5], " Do you want to proceed?");
        assert_eq!(text[6], " ❯ 1. Yes");
        assert_eq!(text[7], "   2. Yes, and don't ask again for: cargo *");
        assert_eq!(text[8], "   3. No");
        assert_eq!(text[9], "");
        assert_eq!(text[10], " Esc to cancel");
        let pointer = rows[6].spans.iter().find(|s| s.content.contains('❯')).unwrap();
        assert_eq!(pointer.style.fg, Some(cc.permission));
    }

    #[test]
    fn a_file_approval_shows_the_path_and_a_dashed_diff() {
        let m = approval(
            AskKind::File,
            "Edit file",
            "README.md",
            "Do you want to make this edit to README.md?",
            vec![Hunk::at(HunkKind::Context, 3, "A tiny project."), Hunk::at(HunkKind::Add, 4, "Edited.")],
        );
        let text: Vec<String> = lines(&m).iter().map(|r| r.to_string().trim_end().to_string()).collect();
        assert_eq!(text[1], " Edit file");
        assert_eq!(text[2], " README.md");
        assert!(text[3].starts_with('╌'));
        assert_eq!(text[4], " 3  A tiny project.");
        assert_eq!(text[5], " 4 +Edited.");
        assert!(text[6].starts_with('╌'));
        assert_eq!(text[7], " Do you want to make this edit to README.md?");
    }
```

davinci_interactive.rs:

```rust
    #[test]
    fn permission_titles_follow_claude_code() {
        assert_eq!(permission_title("bash", true), "Bash command");
        assert_eq!(permission_title("powershell", true), "PowerShell command");
        assert_eq!(permission_title("edit", true), "Edit file");
        assert_eq!(permission_title("write", false), "Create file");
        assert_eq!(permission_title("write", true), "Overwrite file");
        assert_eq!(permission_title("web_fetch", true), "Web fetch");
    }
```

- [ ] **Step 2: Run.** Expected: FAIL.

- [ ] **Step 3: Implement.**

`ask.rs::lines`: when `ask.key == "/permissions"` and `model.approval_instructions` is `None`, return `approval_lines(model)`; keep the denial-instructions editor as is (restyle its title only).

```rust
fn approval_lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let ask = &model.ask;
    let width = model.width;
    let mut rows = vec![Line::from(span("─".repeat(width as usize), cc.permission))];
    let mut title = span(format!(" {}", ask.title), cc.permission);
    title.style = title.style.add_modifier(Modifier::BOLD);
    rows.push(Line::from(title));
    match ask.kind {
        AskKind::Shell => {
            rows.push(blank());
            for line in ask.subject.lines() {
                rows.push(Line::from(truncate_run(vec![span(format!("   {line}"), th.text)], width)));
            }
            rows.push(blank());
        }
        AskKind::File => {
            rows.push(Line::from(span(format!(" {}", ask.subject), cc.inactive)));
            if !ask.preview.is_empty() {
                let dashes = Line::from(span("╌".repeat(width as usize), cc.subtle));
                rows.push(dashes.clone());
                let digits = ask.preview.iter().filter_map(|h| h.line).max().map_or(1, |n| n.to_string().len());
                rows.extend(ask.preview.iter().map(|hunk| approval_hunk(th, hunk, digits, width)));
                rows.push(dashes);
            }
        }
        AskKind::List => {
            if !ask.note.is_empty() {
                rows.push(Line::from(span(format!(" {}", ask.note), cc.inactive)));
            }
            rows.push(blank());
        }
    }
    rows.push(question_line(th, &ask.question, &ask.subject));
    let selected = model.selection(ask.items.len());
    for (index, item) in ask.items.iter().enumerate() {
        let focused = Some(index) == selected;
        let mut row = vec![span(if focused { " ❯ " } else { "   " }, cc.permission)];
        row.push(span(format!("{}. ", index + 1), cc.inactive));
        row.push(span(item.label.clone(), if focused { cc.permission } else { th.text }));
        rows.push(Line::from(truncate_run(row, width)));
    }
    rows.push(blank());
    rows.push(Line::from(span(" Esc to cancel", cc.inactive)));
    rows
}

/// ` Do you want to make this edit to README.md?` with the file name bold.
fn question_line(theme: &Theme, question: &str, subject: &str) -> Line<'static> {
    let name = std::path::Path::new(subject)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    match (!name.is_empty()).then(|| question.find(&name)).flatten() {
        Some(at) => {
            let mut bold = span(name.clone(), theme.text);
            bold.style = bold.style.add_modifier(Modifier::BOLD);
            Line::from(vec![
                span(format!(" {}", &question[..at]), theme.text),
                bold,
                span(question[at + name.len()..].to_string(), theme.text),
            ])
        }
        None => Line::from(span(format!(" {question}"), theme.text)),
    }
}

/// A preview row: ` N sign text` at column 0, as in turn/31-permission-1.
fn approval_hunk(theme: &Theme, hunk: &Hunk, digits: usize, width: u16) -> Line<'static> {
    let cc = theme.cc();
    let number = hunk.line.map_or(String::new(), |n| n.to_string());
    let (sign, fg, bg) = match hunk.kind {
        HunkKind::Add => ("+", cc.diff_add, Some(cc.diff_add_bg)),
        HunkKind::Del => ("-", cc.diff_del, Some(cc.diff_del_bg)),
        HunkKind::Context => (" ", cc.diff_text, None),
    };
    let paint = |fg: Color| match bg {
        Some(bg) => Style::default().fg(fg).bg(bg),
        None => Style::default().fg(fg),
    };
    let mut gutter = paint(fg);
    if bg.is_none() {
        gutter = gutter.add_modifier(Modifier::DIM);
    }
    let text = clip_ellipsis(&hunk.text, width.saturating_sub(digits as u16 + 4));
    let mut spans = vec![
        Span::styled(format!(" {number:>digits$} "), gutter),
        Span::styled(sign.to_string(), paint(fg)),
        Span::styled(text, paint(cc.diff_text)),
    ];
    if let Some(bg) = bg {
        let used = run_width(&spans);
        spans.push(Span::styled(" ".repeat(width.saturating_sub(used) as usize), Style::default().bg(bg)));
    }
    Line::from(spans)
}
```

In `compose` (app.rs) the overlay is drawn where the composer was; confirm the approval rows replace the composer and footer rows (no composer rule above them) by running the existing permission-panel tests and the `approve.py`-style headless check below.

`davinci_interactive.rs::permission_ask`:

```rust
pub fn permission_title(tool: &str, exists: bool) -> String {
    match tool {
        "bash" => "Bash command".into(),
        "powershell" => "PowerShell command".into(),
        "edit" | "notebook_edit" | "apply_patch" => "Edit file".into(),
        "write" if exists => "Overwrite file".into(),
        "write" => "Create file".into(),
        other => {
            let words = other.replace('_', " ");
            let mut chars = words.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => "Tool".into(),
            }
        }
    }
}
```

Build the ask:

`permission_ask(request, trusted)` has no working directory today. Change it to `permission_ask(request, trusted, cwd: &std::path::Path)` and pass `&shell.agent.cwd` (or the agent cwd in scope) at each call site (about 11; `rtk cargo build` lists them). Relative paths must be joined with this cwd, never resolved against the process directory.

```rust
    let file_tool = matches!(request.tool.as_str(), "edit" | "write" | "notebook_edit" | "apply_patch");
    let shell_tool = matches!(request.tool.as_str(), "bash" | "powershell");
    let exists = file_tool && cwd.join(&request.subject).exists();
    let name = std::path::Path::new(&request.subject)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| request.subject.clone());
    let question = match request.tool.as_str() {
        "write" if !exists => format!("Do you want to create {name}?"),
        _ if file_tool => format!("Do you want to make this edit to {name}?"),
        _ => "Do you want to proceed?".to_string(),
    };
```

Items use Claude Code wording, same order and meaning as today:

| Decision | Label |
|---|---|
| `AllowOnce` | `Yes` |
| `AllowForSession` | `Yes, and don't ask again this session for: {rule}` |
| `AllowAlways` | `Yes, and don't ask again for: {rule}` |
| `Deny` | `No` |
| deny with instructions | `No, and tell the model what to do differently` |

`preview` for file tools: `approval_preview(request, cwd)`, reading the call's arguments from `request.args`. The argument names are defined in `crates/davinci-agent/src/tools.rs` (around lines 299 to 321): edit takes `path` with `oldText`/`newText` or an `edits: [...]` array (use the first entry), write takes `path` and `content`. For edit, read the file (`cwd.join(path)`), find the byte offset of the old text, count `\n` before it for the start line, then emit context/`-`/`+` rows numbered from that line. For `write`, emit up to 20 context rows numbered from 1 from the `content` argument. On any read failure return an empty preview (the panel then shows only the path). Keep the `outside the project` note: append ` · outside the project` to `subject` when `request.outside_project`.

`subject` is the command for shell tools and the relative path for file tools.

The `Ask` literals in `permission_ask` and `scope_expansion_ask` (davinci_interactive.rs around 2731 and 2753) list every field; give them the new fields or `..Default::default()`.

Imports for `ask.rs`: `use ratatui::style::{Color, Modifier, Style}; use ratatui::text::Span;`, `blank`, `truncate_run`, `clip_ellipsis` and `run_width` from `crate::davinci::ui`, `crate::davinci::theme::Theme`, and `AskKind`, `Hunk`, `HunkKind` from `crate::davinci::model` (the tests module needs the last three as well).

- [ ] **Step 4: Run** both crates. Update the existing `permission_ask` tests that pinned `allow once`, `Permission · …` titles.
- [ ] **Step 5: Headless check.** Build release, then drive it with `PI_OFFLINE=1` and `PI_OFFLINE_TOOL_CALL='{"name":"bash","arguments":{"command":"git status"}}'` (see the fixture in `main.rs::offline_stub_message`) through `scripts/ui/capture_tui.py` with a one-line change to type a prompt and snap after 3 s; compare with `turn2/41-permission-3`.
- [ ] **Step 6: Commit** `git commit -am "feat(tui): Claude Code permission panel"`

### Task C2: Shared panel rows and titles

**Files:** `crates/davinci-tui/src/davinci/ui.rs` (`section_row`, `hint_row`), `crates/davinci-tui/src/davinci/app.rs` (`command_panel_frame`), `views/chrome.rs` (`effort_rule`).

- [ ] **Step 1: Failing tests** (ui.rs tests)

```rust
    #[test]
    fn a_panel_row_matches_the_reference_picker() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let cc = theme.cc();
        let focused = section_row(100, &theme, true, "1. Default", "");
        let other = section_row(100, &theme, false, "2. Opus", "");
        assert!(focused.to_string().starts_with("❯ 1. Default"), "{focused}");
        assert!(other.to_string().starts_with("  2. Opus"), "{other}");
        let pointer = focused.spans.iter().find(|s| s.content.contains('❯')).unwrap();
        assert_eq!(pointer.style.fg, Some(cc.permission));
        let label = focused.spans.iter().find(|s| s.content.contains("Default")).unwrap();
        assert_eq!(label.style.fg, Some(cc.permission));
        assert_eq!(label.style.bg, None, "no selection band");
    }
```

app.rs (`mod section_regressions`):

```rust
    #[test]
    fn command_panels_use_a_permission_colored_rule_and_title() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 32, false);
        fixtures::dress_screen(&mut model, "3c");
        let frame = compose(&model, 32);
        let cc = model.theme.cc();
        let rule = frame.iter().find(|l| l.to_string().starts_with('▔')).unwrap();
        assert_eq!(rule.spans[0].style.fg, Some(cc.permission));
        let title = frame.iter().find(|l| l.to_string().starts_with("   ") && l.spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD))).unwrap();
        let bold = title.spans.iter().find(|s| s.style.add_modifier.contains(Modifier::BOLD)).unwrap();
        assert_eq!(bold.style.fg, Some(cc.permission));
    }
```

(`3c` is the Thinking sheet, which goes through `command_panel_frame`; `3a` (models) and `3b` (settings) are drawn by `cogitator::screen` and `settings::screen` and never reach it. Add `use ratatui::style::Modifier;` in `mod section_regressions`.)

- [ ] **Step 2: Run.** Expected: FAIL.

- [ ] **Step 3: Implement.**
- `section_row`: marker `❯ ` in `cc.permission` when selected, two spaces otherwise (replace `selection_bar`); no background band; label `cc.permission` when selected else `theme.text`; value `cc.inactive`. Keep the bold on the selected label only if the reference shows it (it does not: remove the bold).
- `hint_row`: every span `cc.inactive`.
- `command_panel_frame`: `chrome::effort_rule` and the plain `▔` rule use `cc.permission` for the `▔` run and `cc.inactive` for the effort text; the title uses `cc.permission` + BOLD via a plain span (not `span_strong` with `th.primary`, which is already `b1b9f9` in dark truecolor but not in light).
- The current value marker: where a sheet marks the saved value (search `✓` or `(current)` in `views/cogitator.rs`, `views/settings.rs`, `views/thinking.rs`), render ` ✔` and color both the label and `✔` `cc.success`.

- [ ] **Step 4: Run** `rtk cargo test -p davinci-tui --offline`. Update picker tests that expect the old selection bar `▌`/copper band.
- [ ] **Step 5: Commit** `git commit -am "feat(tui): Claude Code picker rows and panel titles"`

### Task C3: Model picker details

**Files:** `views/cogitator.rs`.

- [ ] **Step 1: Failing test** (cogitator.rs tests; build the picker model the way its existing tests do)

```rust
    #[test]
    fn the_effort_row_matches_the_reference() {
        let m = model(120); // existing helper: dresses fixture "3a"
        // The effort row is drawn by `picker_panel`, reached through `screen`;
        // `lines(model, config_path)` is the overlay list and has no effort row.
        let rows = screen(&m, 24);
        let cc = m.theme.cc();
        let effort = rows.iter().find(|r| r.to_string().contains("←/→ to adjust")).unwrap();
        assert!(effort.to_string().trim_start().starts_with("● "), "{effort}");
        assert_eq!(effort.spans.iter().find(|s| s.content.contains('●')).unwrap().style.fg, Some(cc.claude));
        assert_eq!(effort.spans.iter().find(|s| s.content.contains("←/→")).unwrap().style.fg, Some(cc.subtle));
    }
```

- [ ] **Step 2: Run.** Expected: FAIL on colors.
- [ ] **Step 3: Implement.** Effort row `   ● {Level} effort` + ` ←/→ to adjust`: `●` `cc.claude`, level text `cc.inactive`, hint `cc.subtle`. Model rows: label column padded to 25 cells as in `ui/19-model-picker` (`1. Default (recommended) ✔  Opus 5.5 · …`: label starts at column 5, detail at column 32); detail text `cc.inactive`. Footer `   Enter to confirm · ←/→ effort · Esc to cancel` in `cc.inactive`.
- [ ] **Step 4: Run** `rtk cargo test -p davinci-tui --offline cogitator`.
- [ ] **Step 5: Commit** `git commit -am "feat(tui): Claude Code model picker details"`

### Task C4: Documentation and delivery

**Files:** `docs/ui/design.md`, `CLAUDE.md`, `docs/ui/claude-code-reference/README.md` (exists; update only if frames change).

- [ ] **Step 1:** Add a section at the top of `docs/ui/design.md`, "Claude Code conversation contract (September 2026)": the default dark theme follows `docs/ui/claude-code-reference/` and this plan's Contract; list the Intentional Differences; state that the editorial print notes below now apply to the optional `vox` theme only.
- [ ] **Step 2:** In `CLAUDE.md`, replace the sentence starting "Its current visual contract is `docs/ui/design.md`: a Claude Code-style conversation with warm coral accents…" with one that names `docs/ui/claude-code-reference/` as the ground truth and mentions Enter-runs-command, the `?` panel and shell mode.
- [ ] **Step 3:** If any frames were captured again during the work, update `docs/ui/claude-code-reference/README.md` (version, date, sets).
- [ ] **Step 4:** Full gate: `rtk cargo fmt --all -- --check`, `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`, `rtk cargo test --workspace --offline`.
- [ ] **Step 5:** Deliver per CLAUDE.md (release build, backup, replace, SHA-256 compare, smoke check). Tell the user to restart open davinci sessions.
- [ ] **Step 6:** Commit, push `ui-cc-c-panels`, open the PR with a side-by-side list of reference frame and davinci capture for: welcome, `/` list, `!` mode, the five footers, a finished turn, a shell approval, an edit approval, the model picker.

## Self-Review Notes

- Every Contract section maps to a task: palette A1; keys A2; completion list A3; composer A4; footer and `?` A5; shell echo A6; user echo B1; markdown B2; tool rows B3; grouping B4; diffs B5; working and done lines B6; reasoning and banner B7; permission panel C1; picker rows C2; model picker C3.
- Not covered on purpose: `/config` tabs and search boxes, `/resume` search box, `/help` tabs (davinci keeps its own sheets for these; C2 restyles their rows and titles), tips row, completion clock, Claude Code's automatic reply after `!` commands.
- Risky spots for the implementer: the caret shift in shell mode (A4 step 3.6), `tail_lines` grouping parity (B4), and Enter routing when a list is open (A2). Each has a test that fails if it breaks.
