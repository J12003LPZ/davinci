defmodule Davinci.Views.Compact do
  @moduledoc """
  4c — `/compact`. What compaction would keep, what it would fold away, and what
  it costs — stated before it happens, like the governor proposal in 2c
  (design.md §6). It never acts silently.

  The cost the screen exists to make visible is the one nobody expects: folding
  the context re-primes the prompt cache, so the next turn pays a full cache
  write before it reads a single new token.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    plan = Model.compaction()

    echo =
      line([
        t(Theme.glyph(:user) <> " ", th.primary),
        t("/compact keep the store.rs decisions verbatim", th.muted)
      ])

    meters = [
      meter_row("now", plan.before_tokens, plan.before_fraction, plan.before_note, th.warning, th),
      meter_row("after", plan.after_tokens, plan.after_fraction, plan.after_note, th.success, th)
    ]

    kept =
      box(
        width: w,
        theme: th,
        title: [t("KEPT VERBATIM", th.success)],
        body:
          Enum.map(plan.kept, fn text ->
            [t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis), t(text, th.muted)]
          end)
      )

    folded =
      box(
        width: w,
        theme: th,
        title: [t("FOLDED INTO ONE NOTE", th.error)],
        right: [t("retrievable by id", th.border)],
        body:
          Enum.map(plan.folded, fn text ->
            [t(Theme.glyph(:failed) <> " ", th.error, nil, th.emphasis), t(text, th.muted)]
          end)
      )

    cost =
      box(
        width: w,
        theme: th,
        border: th.warning,
        title: [t("WHAT THIS COSTS YOU", th.warning)],
        body:
          Enum.map(
            wrap(
              "! compaction re-primes the prompt cache. The next turn pays a full " <>
                "#{plan.after_tokens} cache write before it reads a single new token.",
              w - 6
            ),
            fn row -> [t(row, th.text)] end
          ) ++
            [
              [],
              [
                t("recovers ", th.muted),
                t(plan.recovers, th.success),
                t("   summarising call ", th.muted),
                t(plan.call_cost, th.text),
                t("   cache write ", th.muted),
                t(plan.cache_cost, th.text)
              ],
              [
                t("reversible ", th.muted),
                t(Theme.glyph(:done), th.success, nil, th.emphasis),
                t("  the jsonl keeps every turn", th.border)
              ],
              [],
              [
                t("[enter]", th.primary),
                t(" compact now", th.text),
                t("   [e]", th.primary),
                t(" evict tool output only", th.muted)
              ],
              [
                t("[t]", th.primary),
                t(" raise the threshold", th.muted),
                t("   [esc]", th.border),
                t(" leave it", th.border)
              ]
            ]
      )

    tail = [
      line([
        t("compacted ", th.muted),
        t("2×", th.text),
        t(" this session", th.muted),
        t(" · ", th.border),
        t("last at turn 24, recovered 88k", th.border)
      ]),
      line([t("/tree still shows every folded turn", th.border)])
    ]

    [echo, blank()] ++
      meters ++
      [blank()] ++ kept ++ [blank()] ++ folded ++ [blank()] ++ cost ++ [blank()] ++ tail
  end

  defp meter_row(label, tokens, fraction, note, color, th) do
    left = [
      t(String.pad_trailing(label, 7), th.muted),
      t(String.pad_leading(tokens, 7) <> "  ", th.text)
    ]

    line([left, meter(fraction, 24, th, color), t("  " <> note, note_color(color, th))])
  end

  defp note_color(color, th), do: if(color == th.warning, do: th.warning, else: th.border)
end
