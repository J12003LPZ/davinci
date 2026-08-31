defmodule Davinci.Views.Mensura do
  @moduledoc """
  2c — the token governor. Budget by role, one row each: `role tokens meter
  cap`. Rows within cap use verdigris, the breaching row copper with a warning
  cap note. The proposal always states recovers / keeps / cost / reversible,
  then keyed actions. It never acts silently (design.md §6).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)

    head = [
      line([
        t("in use ", th.muted),
        t("128.4k", th.text),
        t(" of ", th.muted),
        t("200k", th.text)
      ]),
      line([
        t("headroom ", th.muted),
        t("71.6k", th.text),
        t(" · ", th.border),
        t("1.9k tok/s", th.muted),
        t(" · ", th.border),
        t(Theme.pie(0.64) <> " 64%", th.primary)
      ])
    ]

    rows =
      Enum.map(Model.budget(), fn {role, tokens, fraction, note, status} ->
        breach? = status == :breach
        color = if breach?, do: th.primary, else: th.secondary

        left = [
          t(String.pad_trailing(role, 13), if(breach?, do: th.text, else: th.muted)),
          t(String.pad_leading(tokens, 6) <> "  ", color)
        ]

        right = [t(note, if(breach?, do: th.warning, else: th.border))]
        gap = max(w - seg_len(left) - seg_len(right) - 26, 1)

        line([left, meter(fraction, 24, th, color), sp(gap), right])
      end)

    governor =
      box(
        width: w,
        theme: th,
        border: th.warning,
        title: [t("GOVERNOR", th.warning)],
        right: [t("1 proposal", th.border)],
        body:
          Enum.map(
            wrap(
              "! transcript is 19% over its soft cap. Proposed: summarise turns " <>
                "1-18 into one note and evict their tool output.",
              w - 6
            ),
            fn row -> [t(row, th.text)] end
          ) ++
            [
              [],
              [
                t("recovers ", th.muted),
                t("18.2k", th.success),
                t("   keeps ", th.muted),
                t("last 6 turns", th.text),
                t("   cost ", th.muted),
                t("1 summarising call", th.text),
                t("   reversible ", th.muted),
                t(Theme.glyph(:done), th.success, nil, th.emphasis)
              ],
              [],
              [
                t("[a]", th.primary),
                t(" apply", th.text),
                t("   [e]", th.primary),
                t(" evict oldest 6", th.muted),
                t("   [p]", th.primary),
                t(" policy", th.muted),
                t("   [h]", th.primary),
                t(" hold, warn at 90%", th.muted),
                t("   [d]", th.primary),
                t(" dismiss", th.muted)
              ]
            ]
      )

    tail = [
      line([
        t("session spend ", th.muted),
        t("412k", th.text),
        t(" · ", th.border),
        t("daily cap 2m", th.muted),
        t(" · ", th.border),
        t(Theme.pie(0.21) <> " 21%", th.primary)
      ]),
      line([
        t("governor acted 3× today", th.muted),
        t(" · ", th.border),
        t("last: evicted 2 tool results", th.border)
      ])
    ]

    head ++ [blank()] ++ rows ++ [blank()] ++ governor ++ [blank()] ++ tail
  end
end
