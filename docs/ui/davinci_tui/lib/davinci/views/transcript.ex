defmodule Davinci.Views.Transcript do
  @moduledoc """
  The transcript is the interface (design.md §1). No bubbles, no timestamps, no
  decoration in the body: user turns are `> text` in muted, agent turns open
  with `◆ davinci`, tool calls are one line each, and prose wraps at the measure
  even when the terminal is wider.
  """

  import Davinci.Ui

  alias Davinci.Theme
  alias Davinci.Views.Studio

  def lines(model, opts \\ []) do
    th = Keyword.get(opts, :theme, model.theme)
    w = Keyword.get(opts, :width, model.width)
    entries = Keyword.get(opts, :entries, model.transcript)

    entries
    |> Enum.flat_map(&entry(&1, model, th, w))
  end

  defp entry(:gap, _model, _th, _w), do: [blank()]

  defp entry({:user, string}, _model, th, w) do
    [
      line([
        t(Theme.glyph(:user) <> " ", th.primary),
        t(clip(string, w - 4), th.muted)
      ])
    ]
  end

  defp entry({:agent, name}, _model, th, _w) do
    [
      line([
        t(Theme.glyph(:agent) <> " ", th.primary, nil, th.emphasis),
        t(name, th.text)
      ])
    ]
  end

  defp entry({:tool, state, instrument, target, duration}, _model, th, w) do
    [tool_line(w, th, state, instrument, target, duration)]
  end

  defp entry({:detail, string}, _model, th, _w) do
    [detail_line(th, string)]
  end

  defp entry({:prose, string}, _model, th, w) do
    string
    |> wrap(min(measure(), w - 2))
    |> Enum.map(&line([t(&1, th.text)]))
  end

  defp entry({:studio, steps}, model, th, w) do
    Studio.lines(model, th, w, steps)
  end

  defp entry({:delta, path, adds, dels, hunks}, _model, th, w) do
    header =
      line([
        t(Theme.glyph(:delta) <> " ", th.primary),
        t(clip(path, w - 20), th.text),
        t("  +#{adds}", th.success),
        t(" -#{dels}", th.error)
      ])

    [header] ++ Enum.map(hunks, &hunk(&1, th, w))
  end

  # Hunks sit behind a single left rule; no line numbers unless asked.
  defp hunk({kind, string}, th, w) do
    {sign, color} =
      case kind do
        :add -> {"+", th.success}
        :del -> {"-", th.error}
        _ -> {" ", th.muted}
      end

    indent(2, [
      t("│ ", th.border),
      t(sign <> " ", color),
      t(clip(string, w - 10), color)
    ])
  end
end
