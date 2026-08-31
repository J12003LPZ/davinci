defmodule Davinci.Ui do
  @moduledoc """
  Char-grid primitives.

  Ratatouille's `panel` can only color its title and always draws its border in
  the default color, and the design needs bordered surfaces in `border` /
  `primary` / `warning` with a label notched into the top rule and metadata
  notched into the top-right (design.md §3, §6). So panels here are drawn by
  hand out of `label`/`text` runs, which also keeps every surface on the same
  character grid.

  Everything returns either a *segment* (`%Element{tag: :text}`) or a *line*
  (`%Element{tag: :label}`), so callers can count rows exactly and anchor the
  composer to the bottom of the window.
  """

  import Ratatouille.View

  @measure 74

  @doc "Prose never exceeds 74 columns, however wide the terminal (design.md §6)."
  def measure, do: @measure

  # --- segments --------------------------------------------------------------

  def t(content, color \\ nil, background \\ nil, attributes \\ nil) do
    attrs = [content: to_string(content)]
    attrs = if color, do: attrs ++ [color: color], else: attrs
    attrs = if background, do: attrs ++ [background: background], else: attrs
    attrs = if attributes, do: attrs ++ [attributes: attributes], else: attrs
    text(attrs)
  end

  def sp(n, background \\ nil), do: t(String.duplicate(" ", max(n, 0)), nil, background)

  def line(segments), do: label(List.flatten(segments))

  def blank, do: label(content: "")

  def blanks(n) when n > 0, do: List.duplicate(blank(), n)
  def blanks(_), do: []

  def seg_len(segments) do
    segments
    |> List.flatten()
    |> Enum.map(&String.length(Map.get(&1.attributes, :content, "")))
    |> Enum.sum()
  end

  # --- horizontal composition ----------------------------------------------

  @doc "Left run, right run, flush to `width`."
  def spread(width, left, right, background \\ nil) do
    pad = max(1, width - seg_len(left) - seg_len(right))
    line([left, sp(pad, background), right])
  end

  def center(width, segments) do
    pad = max(div(width - seg_len(segments), 2), 0)
    line([sp(pad), segments])
  end

  def indent(n, segments), do: line([sp(n), segments])

  def clip(string, max) when is_binary(string) do
    if String.length(string) > max, do: String.slice(string, 0, max), else: string
  end

  # --- vertical composition -------------------------------------------------

  @doc "Keep the last `n` rows — the transcript scrolls like a terminal."
  def tail(lines, n) when length(lines) > n, do: Enum.drop(lines, length(lines) - n)
  def tail(lines, _n), do: lines

  def pad_to(lines, n), do: lines ++ blanks(n - length(lines))

  @doc "Wrap prose to the measure, returning plain strings."
  def wrap(string, width \\ @measure) do
    string
    |> String.split(~r/\s+/, trim: true)
    |> Enum.reduce([], fn word, acc ->
      case acc do
        [] ->
          [word]

        [current | rest] ->
          if String.length(current) + 1 + String.length(word) <= width,
            do: [current <> " " <> word | rest],
            else: [word, current | rest]
      end
    end)
    |> Enum.reverse()
  end

  # --- meters (design.md §6: meters, not bare numbers) ----------------------

  def meter(fraction, width, theme, color \\ nil) do
    color = color || theme.primary
    filled = fraction |> Kernel.*(width) |> round() |> min(width) |> max(0)

    cond do
      filled == 0 ->
        [t(String.duplicate("─", width), theme.border)]

      true ->
        [
          t(String.duplicate("━", filled - 1), color),
          t("╸", color),
          t(String.duplicate("─", width - filled), theme.border)
        ]
    end
  end

  @doc "The `constructio III / V` tick meter (design.md §6)."
  def ticks(done, total, cell_width, theme) do
    per = max(div(cell_width, total), 1)
    [t(String.duplicate("━", per * done), theme.primary),
     t(String.duplicate("·", per * (total - done)), theme.border)]
  end

  # --- boxes ----------------------------------------------------------------

  @doc """
  A bordered surface with its label notched into the top-left of the rule and
  optional metadata notched into the top-right.

  Options: `:width`, `:theme`, `:border`, `:title`, `:right`, `:body`,
  `:indent`. `:body` is a list of segment-lists; each becomes one row.
  """
  def box(opts) do
    theme = Keyword.fetch!(opts, :theme)
    outer = Keyword.fetch!(opts, :width)
    inset = Keyword.get(opts, :indent, 0)
    width = outer - inset
    border = Keyword.get(opts, :border) || theme.border
    title = Keyword.get(opts, :title, [])
    right = Keyword.get(opts, :right, [])
    body = Keyword.get(opts, :body, [])

    rows =
      [box_top(width, title, right, border)] ++
        Enum.map(body, &box_row(width, &1, border)) ++
        [box_bottom(width, border)]

    if inset == 0, do: rows, else: Enum.map(rows, &prefix(&1, inset))
  end

  @doc "A rule inside a box, used to separate a header row from its list."
  def box_rule(width, theme, indent \\ 0) do
    inner = width - indent - 4
    prefix(line([t("│ ", theme.border), t(String.duplicate("─", inner), theme.border),
                 t(" │", theme.border)]), indent)
  end

  defp prefix(%{tag: :label} = label, n) do
    %{label | children: [sp(n) | label.children]}
  end

  defp box_top(width, title, right, border) do
    left =
      if title == [],
        do: [t("╭─", border)],
        else: [t("╭─ ", border)] ++ List.wrap(title) ++ [t(" ", border)]

    tail =
      if right == [],
        do: [t("─╮", border)],
        else: [t("─ ", border)] ++ List.wrap(right) ++ [t(" ─╮", border)]

    dashes = max(width - seg_len(left) - seg_len(tail), 1)
    line([left, t(String.duplicate("─", dashes), border), tail])
  end

  defp box_row(width, segments, border) do
    inner = width - 4
    pad = max(inner - seg_len(segments), 0)
    line([t("│ ", border), segments, sp(pad), t(" │", border)])
  end

  defp box_bottom(width, border) do
    line([t("╰" <> String.duplicate("─", max(width - 2, 0)) <> "╯", border)])
  end

  @doc "A plain horizontal rule with a mark at its centre (startup, 1a)."
  def hair_rule(width, theme, mark \\ "◦") do
    arm = max(div(width - 4 - String.length(mark) - 2, 2), 1)
    dash = String.duplicate("─", arm)
    line([t("·" <> dash <> " ", theme.border), t(mark, theme.muted),
          t(" " <> dash <> "·", theme.border)])
  end

  # --- transcript building blocks -------------------------------------------

  @doc "`glyph  instrument · verb   target   duration` — one line, no box."
  def tool_line(width, theme, state, instrument, target, duration) do
    glyph = Davinci.Theme.glyph(state)
    color = Davinci.Theme.state_color(theme, state)
    body = min(width - 2, @measure + 4)

    left = [
      t(glyph <> " ", color, nil, theme.emphasis),
      t(instrument, theme.muted),
      t(" · ", theme.border),
      t(target, if(state in [:read, :search], do: theme.secondary, else: theme.muted))
    ]

    right = if duration, do: [t(duration, theme.border)], else: []
    indent(2, [spread_segments(body, left, right)])
  end

  defp spread_segments(width, left, right) do
    pad = max(1, width - seg_len(left) - seg_len(right))
    [left, sp(pad), right]
  end

  def detail_line(theme, string) do
    indent(6, [t(string, theme.muted)])
  end
end
