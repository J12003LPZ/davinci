defmodule Davinci.Views.Recovery do
  @moduledoc """
  6c — what ctrl+c actually did, and what the provider did before it.

  This is a transcript state rather than an instrument, so it opens with the
  turn that failed and keeps its tool lines; the two panels are the exception
  design.md §6 allows for a turn that did not complete. Both state what was
  kept, what was billed, and what is still on disk — the questions someone asks
  in the second after they press ctrl+c.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    run = Model.failed_run()

    turn = [
      line([t(Theme.glyph(:user) <> " ", th.primary), t(run.prompt, th.muted)]),
      blank(),
      line([t(Theme.glyph(:agent) <> " davinci", th.primary)])
    ]

    tools =
      Enum.map(run.tools, fn {state, text, detail} ->
        indent(2, [
          t("⎿ ", th.border),
          t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
          t(text, if(state == :failed, do: th.error, else: th.muted)),
          t("  " <> detail, th.border)
        ])
      end)

    failure =
      box(
        width: w,
        theme: th,
        border: th.error,
        title: [t("THE TURN DID NOT COMPLETE", th.error)],
        body:
          Enum.map(
            wrap(
              "× anthropic returned 429 mid-stream. Retry-After says 12s; this is " <>
                "attempt 2 of 4, backing off 2s, 6s, 12s.",
              w - 6
            ),
            fn row -> [t(row, th.text)] end
          ) ++
            [
              [],
              [
                t("kept ", th.muted),
                t(run.kept, th.success),
                t(" of reply   files written ", th.muted),
                t("0", th.text),
                t("   billed ", th.muted),
                t(run.billed, th.text)
              ],
              [
                t("session written ", th.muted),
                t(Theme.glyph(:done), th.success, nil, th.emphasis)
              ],
              [],
              [
                t(Theme.spinner(model.tick, model.animate) <> " ", th.primary, nil, th.emphasis),
                t("retrying in 9s", th.text),
                t("   [enter]", th.primary),
                t(" retry now", th.text),
                t("   [m]", th.primary),
                t(" finish on opus", th.muted)
              ],
              [
                t("[esc]", th.border),
                t(" stop retrying", th.border)
              ]
            ]
      )

    interrupted =
      box(
        width: w,
        theme: th,
        border: th.warning,
        title: [t("INTERRUPTED", th.warning)],
        body:
          Enum.map(
            wrap(
              "! You stopped the run, not the app. The partial reply stays in the " <>
                "transcript so the next turn can see what it was doing.",
              w - 6
            ),
            fn row -> [t(row, th.text)] end
          ) ++
            [[]] ++
            Enum.map(run.aftermath, fn {state, text} ->
              [
                t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
                t(text, th.muted)
              ]
            end)
      )

    tail =
      Enum.map(
        wrap(
          "Say continue and it picks up from the partial reply; give it a " <>
            "different instruction and the partial is context, not a commitment.",
          measure()
        ),
        fn row -> line([t(row, th.text)]) end
      )

    turn ++
      tools ++
      [blank()] ++
      failure ++
      [blank(), line([t(Theme.glyph(:user) <> " ", th.primary), t("ctrl+c", th.border)]), blank()] ++
      interrupted ++ [blank()] ++ tail
  end
end
