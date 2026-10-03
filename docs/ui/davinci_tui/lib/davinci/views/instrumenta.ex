defmodule Davinci.Views.Instrumenta do
  @moduledoc """
  1d — the command palette, an inset overlay over a dimmed transcript. Selection
  is marked by a 3-cell copper left bar *plus* a tinted row, so it reads without
  color. The footer states the corpus (design.md §6).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def lines(model) do
    th = model.theme
    inset = if Model.minimal?(model), do: 0, else: 6
    w = model.width
    inner = w - inset - 4
    hits = Model.filtered_corpus(model)
    selected = if hits == [], do: -1, else: rem(model.palette_index, length(hits))

    query = [
      [
        t(Theme.glyph(:prompt) <> " ", th.primary),
        t(model.query, th.text),
        if(Model.blink?(model),
          do: t(" ", th.background, th.primary),
          else: t(" ", th.background, th.background)
        )
      ]
      |> then(fn left ->
        right = [t("#{length(hits)} of #{Model.corpus_total()}", th.border)]
        [left, sp(max(inner - seg_len(left) - seg_len(right), 1)), right]
      end)
    ]

    rows =
      hits
      |> Enum.with_index()
      |> Enum.map(fn {{name, desc, kind}, index} ->
        row(th, inner, name, desc, kind, index == selected)
      end)

    empty =
      if hits == [], do: [[t("no instrument matches that query", th.muted)]], else: []

    footer = [
      [
        t("↑↓ move", th.border),
        t(" · ", th.border),
        t("enter run", th.border),
        t(" · ", th.border),
        t("tab complete", th.border),
        t(" · ", th.border),
        t("esc close", th.border)
      ],
      [
        t("fuzzy: ", th.border),
        t("tools", th.muted),
        t(" · ", th.border),
        t("sessions", th.muted),
        t(" · ", th.border),
        t("files", th.muted),
        t(" · ", th.border),
        t("modes", th.muted)
      ]
    ]

    box(
      width: w,
      indent: inset,
      theme: th,
      title: [t("INSTRUMENTA", th.primary)],
      right: [t("ctrl+p", th.border)],
      body: query ++ [[]] ++ rows ++ empty ++ [[]] ++ footer
    )
  end

  defp row(th, inner, name, desc, kind, selected?) do
    bg = if selected?, do: th.surface, else: nil

    bar =
      if selected?,
        do: t("▌  ", th.primary, bg),
        else: t("   ", nil, bg)

    left = [bar, t(name, if(selected?, do: th.text, else: th.muted), bg)]
    right = [t(kind, th.border, bg)]
    mid = [t(desc, th.muted, bg)]

    name_col = max(div(inner, 3), 24)
    pad_name = max(name_col - seg_len(left), 1)
    pad_mid = max(inner - name_col - seg_len(mid) - seg_len(right), 1)

    [left, sp(pad_name, bg), mid, sp(pad_mid, bg), right]
  end
end
