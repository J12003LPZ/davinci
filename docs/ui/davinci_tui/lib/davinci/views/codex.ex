defmodule Davinci.Views.Codex do
  @moduledoc """
  1e — the workspace sidebar. The only persistent split in the product, opt-in
  at ≥120 columns (design.md §1, §7). At ≥150 columns the git changes popover is
  allowed under the transcript.
  """

  import Ratatouille.View
  import Davinci.Ui

  alias Davinci.{Model, Theme}
  alias Davinci.Views.Transcript

  @doc "One `row` element occupying `rows` terminal rows."
  def render(model, rows) do
    side = 3
    main = 12 - side
    side_w = div(model.width * side, 12)
    main_w = model.width - side_w - 2

    row([
      column([size: side], pad_to(tail(sidebar(model, side_w), rows), rows)),
      column([size: main], pad_to(tail(main_column(model, main_w), rows), rows))
    ])
  end

  defp sidebar(model, w) do
    th = model.theme

    body =
      Enum.map(Model.tree(), fn {depth, twisty, name, status} ->
        [
          sp(depth * 2),
          t(if(twisty, do: twisty <> " ", else: "  "), th.border),
          t(clip(name, w - 12), if(depth == 0, do: th.text, else: th.muted))
        ] ++ status_seg(status, th)
      end)

    footer = [
      [
        t("ctrl+e close", th.border),
        t(" · ", th.border),
        t("/ filter", th.border)
      ]
    ]

    box(
      width: w,
      theme: th,
      title: [t("CODEX", th.primary), t(" · ", th.border), t("WORKSPACE", th.muted)],
      body: body ++ [[]] ++ footer
    )
  end

  defp status_seg(nil, _th), do: []

  defp status_seg(:delta, th), do: [t(" " <> Theme.glyph(:delta), th.primary)]

  defp status_seg(:failed, th),
    do: [t(" " <> Theme.glyph(:failed), th.error, nil, th.emphasis)]

  defp main_column(model, w) do
    transcript =
      Transcript.lines(model, entries: Model.codex_transcript(), width: w)

    if Model.wide?(model), do: transcript ++ [blank()] ++ changes(model, w), else: transcript
  end

  defp changes(model, w) do
    th = model.theme
    box_w = min(w, 46)

    body =
      Enum.map(Model.changes_list(), fn {status, path, count} ->
        color = if status == "A", do: th.success, else: th.warning

        [
          t(status <> "  ", color, nil, th.emphasis),
          t(clip(path, box_w - 14), th.muted),
          sp(max(box_w - 12 - String.length(clip(path, box_w - 14)), 1)),
          t(count, th.success)
        ]
      end)

    box(
      width: box_w,
      theme: th,
      title: [t("CHANGES", th.primary)],
      right: [t("3 files", th.border)],
      body: body
    )
  end
end
