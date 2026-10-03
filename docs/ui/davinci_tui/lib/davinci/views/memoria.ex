defmodule Davinci.Views.Memoria do
  @moduledoc """
  1f — session picker (overlay over a dimmed transcript).
  2b — vector recall: each hit is two rows (score + summary + location, then a
  proportion meter and provenance), hits below the relevance floor are shown as
  held back with their count, so retrieval is auditable (design.md §6).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  # --- 1f: sessions ---------------------------------------------------------

  def sessions(model) do
    th = model.theme
    inset = if Model.minimal?(model), do: 0, else: 8
    w = model.width
    inner = w - inset - 4
    list = Model.sessions()
    selected = rem(model.session_index, length(list))

    rows =
      list
      |> Enum.with_index()
      |> Enum.map(fn {{name, age}, index} ->
        state = if index == selected, do: :active, else: :queued
        bg = if index == selected, do: th.surface, else: nil

        left = [
          t(Theme.glyph(state) <> " ", Theme.state_color(th, state), bg, th.emphasis),
          t(name, if(index == selected, do: th.text, else: th.muted), bg)
        ]

        right = [t(age, th.border, bg)]
        [left, sp(max(inner - seg_len(left) - seg_len(right), 1), bg), right]
      end)

    footer = [
      [
        t("42 turns", th.muted),
        t(" │ ", th.border),
        t("128k tokens", th.muted),
        t(" │ ", th.border),
        t("forked from ", th.muted),
        t("provider-parity", th.secondary)
      ],
      [
        t("enter resume", th.border),
        t(" · ", th.border),
        t("d delete", th.border),
        t(" · ", th.border),
        t("f fork", th.border),
        t(" · ", th.border),
        t("ctrl+s close", th.border)
      ]
    ]

    box(
      width: w,
      indent: inset,
      theme: th,
      title: [t("MEMORIA", th.primary), t(" · ", th.border), t("SESSIONS", th.muted)],
      right: [t("ctrl+s", th.border)],
      body: rows ++ [[]] ++ footer
    )
  end

  # --- 2b: vector recall ----------------------------------------------------

  @floor 0.70

  def recall(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    hits = Model.recall()
    selected = rem(model.recall_index, length(hits))
    held = Enum.count(hits, fn {_s, _t, _l, above, _p} -> is_nil(above) end)

    query =
      box(
        width: w,
        theme: th,
        border: th.secondary,
        title: [t("MEMORIA", th.secondary), t(" · ", th.border), t("RECALL", th.muted)],
        right: [
          t("cosine", th.border),
          t(" · ", th.muted),
          t("12ms", th.border),
          t(" · ", th.muted),
          t("k=6", th.border)
        ],
        body: [
          [
            t(Theme.glyph(:search) <> " ", th.secondary),
            t("how does session persistence survive an interrupt", th.text)
          ]
        ]
      )

    rows =
      hits
      |> Enum.with_index()
      |> Enum.flat_map(fn {hit, index} -> hit_rows(hit, index == selected, th, w) end)

    ledger = [
      line([
        t("promoted to context ", th.muted),
        t("3 chunks", th.text),
        t(" · ", th.border),
        t("1.2k tokens", th.text)
      ]),
      line([
        t("held back ", th.muted),
        t("#{held}", th.warning),
        t(" · ", th.border),
        t("below #{:erlang.float_to_binary(@floor, decimals: 2)} relevance floor", th.muted)
      ]),
      line([
        t("index freshness ", th.muted),
        t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis),
        t("reindexed on last commit", th.muted)
      ])
    ]

    projection = if Model.decoration?(model), do: [blank()] ++ projection(th, w), else: []

    # Exits live in the composer hint line (Chrome.composer/2, hint: :recall).
    query ++ [blank()] ++ rows ++ [blank()] ++ ledger ++ projection
  end

  defp hit_rows({score, summary, location, above, provenance}, selected?, th, w) do
    bg = if selected?, do: th.surface, else: nil
    score_color = if selected?, do: th.primary, else: th.muted

    left = [
      if(selected?, do: t("▌ ", th.primary, bg), else: t("  ", nil, bg)),
      t(score <> "  ", score_color, bg),
      t(clip(summary, w - 40), if(selected?, do: th.text, else: th.muted), bg)
    ]

    right = [t(clip(location, 34), th.muted, bg)]
    head = line([left, sp(max(w - seg_len(left) - seg_len(right), 1), bg), right])

    case above do
      nil ->
        [head]

      fraction ->
        [
          head,
          line(
            [sp(8)] ++
              meter(fraction, 20, th, if(selected?, do: th.primary, else: th.muted)) ++
              [t("  " <> provenance, th.border)]
          )
        ]
    end
  end

  # Decoration with a job: the query against the session cluster (design.md §6).
  @field [
    "  ·   ·      ·        ·   ·",
    "    ·    ·   ·   ·    ·  ·",
    "  ·  · · ·      ·  ·   ·  ·",
    "     ·   ·     · ·  ·      ·",
    "  ·    ·   ·     ·   ·  ·  ·"
  ]

  defp projection(th, w) do
    box(
      width: min(w, 46),
      theme: th,
      title: [t("PROJECTION", th.muted)],
      body:
        Enum.map(Enum.with_index(@field), fn {row, index} ->
          if index == 1 do
            [head, tail] = String.split(row, "·", parts: 2)
            [t(head, th.border), t("◉", th.primary, nil, th.emphasis), t(tail, th.border)]
          else
            [t(row, th.border)]
          end
        end) ++
          [
            [],
            [
              t("session cluster", th.muted),
              t("  ", nil),
              t("◉ ", th.primary),
              t("query", th.text),
              t(" · ", th.border),
              t("18k pts sampled", th.border)
            ]
          ]
    )
  end
end
