defmodule Davinci.Views.Disegno do
  @moduledoc """
  1c — the plan sheet. Roman numerals in a 4-column gutter, a footer that reads
  `constructio III / V` with a tick meter, and one decorative compass in the
  top-right that is clipped by its own layer, so the panel label is never cut
  (design.md §6).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @compass [
    "   ·─────·",
    " ╭─╲  │  ╱─╮",
    "─┼───┼───┼─",
    " ╰─╱  │  ╲─╯",
    "   ·─────·"
  ]

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 12)
    plan = Model.plan()
    done = Enum.count(plan, fn {_n, state, _v, _t} -> state == :done end)
    inner = w - 4

    steps =
      plan
      |> Enum.with_index()
      |> Enum.map(fn {{numeral, state, verb, target}, index} ->
        left =
          [
            t(String.pad_leading(numeral, 3) <> " ", th.border),
            t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
            t(verb, if(state == :queued, do: th.muted, else: th.text))
          ] ++
            if target, do: [t(" · ", th.border), t(target, th.secondary)], else: []

        right =
          if Model.decoration?(model),
            do: [t(Enum.at(@compass, index, ""), th.border)],
            else: []

        pad = max(inner - seg_len(left) - seg_len(right), 1)
        [left, sp(pad), right]
      end)

    footer = [
      [
        t("constructio ", th.muted),
        t("III", th.text),
        t(" / ", th.border),
        t("V  ", th.muted)
      ] ++ ticks(done, length(plan), 24, th),
      [
        t("a accept", th.border),
        t(" · ", th.border),
        t("e edit step", th.border),
        t(" · ", th.border),
        t("ctrl+p instrumenta", th.border)
      ]
    ]

    [
      line([
        t(Theme.glyph(:user) <> " ", th.primary),
        t("add streaming to the anthropic adapter, keep TS parity", th.muted)
      ]),
      blank(),
      line([
        t(Theme.glyph(:agent) <> " ", th.primary, nil, th.emphasis),
        t("davinci", th.text)
      ]),
      blank()
    ] ++
      box(
        width: w,
        theme: th,
        title: [t("DISEGNO", th.primary), t(" · ", th.border), t("IMPLEMENTATION PLAN", th.muted)],
        body: steps ++ [[]] ++ footer
      )
  end
end
