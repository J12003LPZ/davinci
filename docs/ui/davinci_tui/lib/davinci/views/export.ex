defmodule Davinci.Views.Export do
  @moduledoc """
  4d — `/export` and `/share`. A session leaving the machine, with a ledger of
  what goes with it.

  The screen's job is the second column: what was redacted, and what was kept
  that names you anyway — absolute paths, branch names, commit subjects. A
  secret gist is not a private one, and the panel says so rather than implying
  it with a colour.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    ledger = Model.export_ledger()

    echo =
      line([
        t(Theme.glyph(:user) <> " ", th.primary),
        t("/export review-agent-runtime.html", th.muted)
      ])

    formats =
      line([
        t("format ", th.border),
        t(" .html ", th.background, th.primary),
        t(" ", th.border),
        t(" .jsonl ", th.muted),
        t(" gist ", th.muted),
        t("   one page, no assets, opens offline", th.border)
      ])

    included =
      Enum.map(ledger.included, fn text ->
        [t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis), t(text, th.muted)]
      end)

    excluded =
      Enum.map(ledger.excluded, fn {state, text} ->
        [
          t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
          t(text, th.muted)
        ]
      end)

    written =
      box(
        width: w,
        theme: th,
        title: [t("WHAT LEAVES THE SESSION", th.primary)],
        right: [t(ledger.size, th.border)],
        body:
          included ++
            [[]] ++
            excluded ++
            [
              [],
              [t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis), t("wrote 42 of 42 turns", th.text)] ++
                [t("  ", th.border)] ++
                meter(1.0, 20, th, th.success) ++
                [t("  " <> ledger.elapsed, th.border)]
            ]
      )

    share =
      box(
        width: w,
        theme: th,
        border: th.secondary,
        title: [t("SHARE", th.secondary), t(" · ", th.border), t("SECRET GIST", th.muted)],
        body:
          [
            [
              t(Theme.glyph(:read) <> " ", th.secondary),
              t("uploaded to your GitHub account", th.muted),
              t("  " <> ledger.size, th.border)
            ],
            [
              t(Theme.glyph(:read) <> " ", th.secondary),
              t(ledger.gist, th.text),
              t("  copied to the clipboard", th.border)
            ]
          ] ++
            Enum.map(
              wrap(
                "! secret is not private — anyone with the link can read the whole " <>
                  "session",
                w - 6
              ),
              fn row -> [t(row, th.warning)] end
            ) ++
            [
              [],
              [
                t("[o]", th.primary),
                t(" open in browser", th.text),
                t("   [c]", th.primary),
                t(" copy link again", th.muted),
                t("   [d]", th.primary),
                t(" delete the gist", th.muted),
                t("   [esc]", th.border),
                t(" done", th.border)
              ]
            ]
      )

    tail = [
      line([
        t(".jsonl round-trips", th.muted),
        t(" · ", th.border),
        t("/import", th.text),
        t(" resumes it on any machine", th.muted)
      ]),
      line([t("exports are written next to the cwd, never into the session store", th.border)])
    ]

    [echo, blank(), formats, blank()] ++ written ++ [blank()] ++ share ++ [blank()] ++ tail
  end
end
