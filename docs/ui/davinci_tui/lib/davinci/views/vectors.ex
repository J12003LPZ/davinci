defmodule Davinci.Views.Vectors do
  @moduledoc """
  5b — `memory-status`. The vector index itself, as opposed to 2b, which is one
  query against it.

  Records are grouped by the kind the extractor assigned, because that is what
  decides both importance and what gets evicted first; each row is a count with
  its share of the index, never a bare number (design.md §9). The destructive
  action says what it would destroy and that it cannot be undone.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @kind 13

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    index = Model.vector_index()

    head = [
      line([
        t("this repository ", th.muted),
        t(index.repo, th.secondary),
        t(" holds ", th.muted),
        t(index.repo_records, th.primary),
        t(" of ", th.muted),
        t(index.total_records, th.text),
        t(" records", th.muted)
      ]),
      line([
        t("retrieval automatic", th.muted),
        t(" · ", th.border),
        t("at most ", th.muted),
        t(index.injection_cap, th.text),
        t(" injected per turn", th.muted),
        t(" · ", th.border),
        t("floor ", th.muted),
        t(index.floor, th.text)
      ])
    ]

    rows =
      Enum.map(index.kinds, fn {kind, count, fraction, note} ->
        line([
          t(String.pad_trailing(kind, @kind), th.muted),
          t(String.pad_leading(count, 6) <> "  ", th.text)
        ] ++
          meter(fraction, 22, th, th.secondary) ++
          [t("  " <> note, th.border)])
      end)

    where =
      box(
        width: w,
        theme: th,
        title: [t("WHERE IT LIVES", th.secondary)],
        body: [
          [t("embeddings ", th.muted), t(index.embeddings, th.text), t("  " <> index.embed_host, th.border)],
          [t("vectors    ", th.muted), t(index.store, th.text), t("  " <> index.collection, th.border)],
          [t("extraction ", th.muted), t(index.extraction, th.text), t("  one call per turn, off the critical path", th.border)],
          [t("config     ", th.muted), t(index.config, th.border)]
        ]
      )

    health =
      box(
        width: w,
        theme: th,
        title: [t("HEALTH", th.secondary)],
        body:
          Enum.map(index.health, fn {state, text} ->
            [
              t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
              t(text, th.muted)
            ]
          end)
      )

    footer =
      Enum.map(
        wrap(
          "memory-clear drops this repository's #{index.repo_records} records. It " <>
            "asks first, and it cannot be undone.",
          measure()
        ),
        fn row -> line([t(row, th.warning)]) end
      ) ++
        [
          line([
            t("enter search", th.border),
            t(" · ", th.border),
            t("i reindex", th.border),
            t(" · ", th.border),
            t("t toggle automatic retrieval", th.border),
            t(" · ", th.border),
            t("esc close", th.border)
          ])
        ]

    head ++ [blank()] ++ rows ++ [blank()] ++ where ++ [blank()] ++ health ++ [blank()] ++ footer
  end
end
