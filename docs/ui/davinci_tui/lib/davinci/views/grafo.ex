defmodule Davinci.Views.Grafo do
  @moduledoc """
  2a — the code graph. The graph is drawn on a strict column grid: the parent
  connector column is inherited by every child row and no vertical descends
  through label text. Below it, an impact list; untested edges in warning
  (design.md §6).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  # Connector columns are literal so the grid can be verified by eye.
  @graph [
    [{"davinci-cli ", :muted}, {"──┬── ", :border}, {"davinci-agent ", :muted},
     {"──┬── ", :border}, {"davinci-ai ", :muted}, {"─── ", :border},
     {"providers", :muted}],
    [{"              ", :muted}, {"│", :border}, {"                   ", :muted},
     {"├── ", :border}, {"davinci-tools", :muted}],
    [{"              ", :muted}, {"│", :border}, {"                   ", :muted},
     {"└── ", :border}, {"davinci-session ◉", :primary}, {" ── ", :border},
     {"davinci-store", :muted}],
    [{"              ", :muted}, {"└── ", :border}, {"davinci-tui", :muted}]
  ]

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 16)

    graph =
      Enum.map(@graph, fn row ->
        Enum.map(row, fn {string, token} -> t(string, Map.fetch!(th, token), nil,
          if(token == :primary, do: th.emphasis, else: nil)) end)
      end)

    study =
      box(
        width: w,
        theme: th,
        title: [t("GRAFO", th.primary), t(" · ", th.border), t("DEPENDENCY STUDY", th.muted)],
        right: [
          t("412 nodes", th.muted),
          t(" · ", th.border),
          t("1207 edges", th.muted),
          t(" · ", th.border),
          t("0 cycles", th.success)
        ],
        body: graph
      )

    header =
      spread(
        w,
        [t("impact of ", th.text), t("store.rs", th.secondary)],
        [
          t("fan-in ", th.muted),
          t("6", th.text),
          t(" · ", th.border),
          t("fan-out ", th.muted),
          t("2", th.text),
          t(" · ", th.border),
          t("depth ", th.muted),
          t("3", th.text)
        ]
      )

    rows =
      Enum.map(Model.impact(), fn {state, symbol, distance, sites, sites_token} ->
        left = [
          t(Theme.glyph(state) <> "  ", Theme.state_color(th, state), nil, th.emphasis),
          t(symbol, if(state == :active, do: th.text, else: th.muted))
        ]

        right = [
          t(String.pad_trailing(distance, 8), th.muted),
          t(sites, Map.fetch!(th, sites_token))
        ]

        spread(w, left, right)
      end)

    note =
      line([
        t("14 tests touch this path", th.muted),
        t(" · ", th.border),
        t("2 untested edges", th.warning),
        t(" · ", th.border),
        t("graph rebuilt 4s ago from rust-analyzer", th.muted)
      ])

    [
      line([
        t(Theme.glyph(:user) <> " ", th.primary),
        t("/graph impact crates\\davinci-session\\src\\store.rs", th.muted)
      ]),
      blank()
    ] ++
      study ++
      [blank(), header, line([t(String.duplicate("─", w), th.border)])] ++
      rows ++
      [line([t(String.duplicate("─", w), th.border)]), note]
  end
end
