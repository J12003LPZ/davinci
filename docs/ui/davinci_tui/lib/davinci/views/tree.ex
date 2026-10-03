defmodule Davinci.Views.Tree do
  @moduledoc """
  4b — `/tree`. The session as it actually is: a tree, with the forks that were
  abandoned still on it.

  The graph rules from 2a hold here (design.md §6): the trunk column a child
  inherits is drawn for every row of that child, and no vertical ever descends
  through label text — the trunk is built as its own run of segments before the
  glyph, never interleaved with the label.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    list = Model.session_tree()
    selected = rem(model.tree_index, Enum.count(list, & &1.id))

    filters =
      line([
        t("filter ", th.border),
        t(" all ", th.background, th.primary),
        t(" ", th.border),
        t(" no tools ", th.muted),
        t(" user only ", th.muted),
        t(" labeled ", th.muted)
      ])

    tree =
      box(
        width: w,
        theme: th,
        title: [t("MEMORIA", th.primary), t(" · ", th.border), t("SESSION TREE", th.muted)],
        right: [
          t("3 branches", th.muted),
          t(" · ", th.border),
          t(Theme.glyph(:done) <> " nothing lost", th.success)
        ],
        body: rows(list, selected, w, th)
      )

    detail =
      [
        line([
          t("turn ", th.muted),
          t(current(list, selected).id, th.primary),
          t(" · context at this point ", th.muted),
          t("47k/200k", th.text),
          t(" · cost so far ", th.muted),
          t("$0.84", th.text)
        ]),
        line([
          t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis),
          t("4 user turns, 4 agent turns, 11 tool results", th.muted),
          t(" · nothing compacted yet", th.border)
        ]),
        line([
          t(Theme.glyph(:read) <> " ", th.secondary),
          t("the working tree is ahead of this turn", th.muted),
          t(" · the tree never moves your files", th.border)
        ]),
        line([
          t(Theme.glyph(:attention) <> " ", th.warning, nil, th.emphasis),
          t("branch 06 has its own 9 turns and will not merge back", th.muted)
        ])
      ]

    footer = [
      line([
        t("↑↓ move", th.border),
        t(" · ", th.border),
        t("enter switch to turn", th.border),
        t(" · ", th.border),
        t("ctrl+←/→ fold", th.border),
        t(" · ", th.border),
        t("f fork here", th.border),
        t(" · ", th.border),
        t("esc close", th.border)
      ])
    ]

    [filters, blank()] ++ tree ++ [blank()] ++ detail ++ [blank()] ++ footer
  end

  # Rows carry their own trunk, so a spacer row keeps the verticals continuous
  # without any row having to know what came before it.
  defp rows(list, selected, w, th) do
    list
    |> Enum.reduce({[], -1}, fn entry, {acc, index} ->
      case entry.id do
        nil -> {acc ++ [[t(entry.trunk, th.border)]], index}
        _ -> {acc ++ [row(entry, index + 1 == selected, w, th)], index + 1}
      end
    end)
    |> elem(0)
  end

  # Returns a segment list, not a line: these rows are the body of a box, and
  # `Ui.box/1` draws the border around each one.
  defp row(entry, selected?, w, th) do
    state = if selected?, do: :active, else: entry.state
    color = Theme.state_color(th, state)
    text_color = if selected?, do: th.text, else: entry.text_color || th.muted

    left = [
      t(entry.trunk, th.border),
      t(Theme.glyph(state) <> " ", color, nil, th.emphasis),
      t(entry.id <> "  ", th.border),
      t(clip(entry.label, 40), text_color)
    ]

    right =
      cond do
        selected? -> [t(Theme.glyph(:read) <> " here", th.primary)]
        entry.meta -> [t(entry.meta, th.border)]
        true -> []
      end

    pad = max(w - 4 - seg_len(left) - seg_len(right), 1)
    left ++ [sp(pad)] ++ right
  end

  defp current(list, selected) do
    list
    |> Enum.filter(& &1.id)
    |> Enum.at(selected)
  end
end
