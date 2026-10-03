defmodule Davinci.Views.Thinking do
  @moduledoc """
  3c — `/thinking`. Seven levels, each a budget with a meter and a cap, never a
  bare adjective (design.md §9). The meters are scaled to the 64k ceiling, not
  to the window; the `max` row says what fraction of the window it would take,
  because that is the number that hurts.

  The table beneath states what a level actually becomes at each provider: a
  budget in tokens, an effort enum, or nothing at all.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @level 9
  @budget 7

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    list = Model.thinking_levels()
    selected = rem(model.thinking_index, length(list))
    current = Enum.at(list, selected)

    head = [
      line([
        t("thinking ", th.muted),
        t(current.level, th.primary),
        t(" for this turn and the next", th.muted)
      ]),
      line([
        t("every level is a cap, not a promise", th.muted),
        t(" · ", th.border),
        t("the model may think less", th.border)
      ])
    ]

    header =
      line([
        sp(2),
        t(String.pad_trailing("LEVEL", @level + 1), th.border),
        t(String.pad_leading("BUDGET", @budget) <> "  ", th.border),
        t(String.pad_trailing("SHARE OF THE 64k CEILING", 26), th.border),
        t("SONNET → GPT", th.border)
      ])

    rows =
      list
      |> Enum.with_index()
      |> Enum.map(fn {level, index} -> row(level, index == selected, th) end)

    explains =
      box(
        width: w,
        theme: th,
        title: [t("WHAT THE LEVEL DOES", th.primary)],
        body:
          Enum.map(
            [
              "anthropic  sent as a thinking budget in tokens, deducted from the " <>
                "same window as the transcript.",
              "openai  mapped to reasoning effort; seven levels collapse to four, " <>
                "so xhigh and high send the same request.",
              "google  sent as a thinking budget; off means the field is omitted, " <>
                "not zeroed.",
              "a model with no thinking knob keeps the level and ignores it — the " <>
                "status bar says ○ none."
            ]
            |> Enum.flat_map(&wrap(&1, w - 6)),
            fn row -> [t(row, th.muted)] end
          )
      )

    tail = [
      line([
        t("last turn thought ", th.muted),
        t("5.1k", th.text),
        t(" of ", th.muted),
        t("8k", th.text),
        t("  ", nil),
        t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis),
        t("under budget", th.muted)
      ]),
      line([
        t("thinking is billed as output", th.muted),
        t(" · ", th.border),
        t(Theme.pie(0.38) <> " 38%", th.primary),
        t(" of this session's output tokens", th.muted)
      ])
    ]

    head ++ [blank(), header] ++ rows ++ [blank()] ++ explains ++ [blank()] ++ tail
  end

  # No background band on the selected row: Ui.meter/4 draws on the default
  # ground, so a band would tear across the 24 columns of the meter. The glyph
  # and the text ramp carry the selection instead (design.md §4).
  defp row(level, selected?, th) do
    state =
      cond do
        selected? -> :active
        level.warn -> :attention
        true -> :queued
      end

    # The meter is a magnitude, not a state, so it never takes verdigris.
    color =
      cond do
        selected? -> th.primary
        level.warn -> th.warning
        true -> th.muted
      end

    text_color = if selected?, do: th.text, else: th.muted

    left = [
      t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
      t(String.pad_trailing(level.level, @level + 1), text_color),
      t(String.pad_leading(level.budget, @budget) <> "  ", text_color)
    ]

    note = [t("  " <> level.maps_to, if(level.warn, do: th.warning, else: th.border))]

    line([left, meter(level.fraction, 24, th, color), note])
  end
end
