# Claude Code reference frames

Screens captured from Claude Code 2.1.281 on 2026-09-24, used as the ground
truth for davinci's conversation UI (plan:
`docs/superpowers/plans/2026-09-24-claude-code-terminal-ui.md`).

## How they were captured

- Windows 11, pseudo console from `pywinpty`, screen replayed with `pyte`.
- 120 columns by 40 rows, `COLORTERM=truecolor`, `FORCE_COLOR=3`, dark theme.
- Hooks disabled with `--settings '{"disableAllHooks": true}'`.
- A small git repository with `README.md`, `Cargo.toml` and `src/main.rs`.

Each frame has two files:

- `<frame>.txt`: the plain 40-row grid.
- `<frame>.json`: one list of style runs per row. A run is
  `{"text", "fg", "bg"?, "bold"?, "italic"?, "dim"?, "strike"?, "reverse"?}`.
  Colors are hex strings or ANSI names; `default` is the terminal color.
  Blank cells without a background are left out, so count columns in the
  `.txt` file.

## Sets

| Folder | Frames |
|---|---|
| `ui` | welcome, `?` shortcuts, `/` list, `/comp`, `/mo`, `@` list, `!` shell mode, typed and multi-line drafts, the five permission modes, `/model`, `/config`, `/permissions`, `/resume`, `/help`, double Esc, Enter on `/comp` |
| `turn` | one model turn: working line frames, grouped calls, edit approval, diff, markdown reply, completion line, verbose view (`ctrl+o`) |
| `turn2` | create-file and shell approvals, replace diff, write result, working line with a tip row |
| `shell` | `!` command echo and output, `(No output)`, Tab on `/`, Tab on `/mod`, Enter with `@` |

The `ui` set was captured before faint text (SGR 2) was recorded, so its
JSON has no `dim` flags; `turn/01-welcome` shows the dim placeholder.

## Sanitizing

Local paths are replaced with `~\project` or `C:\Users\<user>\…`, the plan
tier with `<plan>`, and the Claude Code logo cells with spaces. Model replies
are left as captured.

## Tools

```powershell
pip install pywinpty pyte
python scripts/ui/capture_tui.py --phase ui --out <dir> --sandbox <git repo>
python scripts/ui/capture_tui.py --phase ui --bin (Get-Command davinci).Source --out <dir> --sandbox <git repo>
python scripts/ui/show_styles.py docs/ui/claude-code-reference/turn 34-final
```

The `turn` phase sends real prompts and spends model usage. davinci writes no
color codes inside this pseudo console, so compare its captures as text only.
