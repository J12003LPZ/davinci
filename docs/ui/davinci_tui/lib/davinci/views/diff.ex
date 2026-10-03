defmodule Davinci.Views.Diff do
  @moduledoc """
  6d — the Δ review: every file the turn touched, before you keep any of it.

  The file list carries what the change did to the tests, because "+21 −6" and
  "no test covers this path" are two different pieces of news. The selected file
  expands to its hunks behind a single left rule, additions in success,
  deletions in error, context muted — the Δ block of design.md §6 at full size.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @path 44
  @tests 19

  def lines(model) do
    th = model.theme
    review = Model.review()
    selected = rem(model.diff_index, length(review.files))
    current = Enum.at(review.files, selected)

    head =
      line([
        t("#{length(review.files)} files", th.muted),
        t("   ", th.border),
        t("+#{review.adds}", th.success),
        t(" ", th.border),
        t("-#{review.dels}", th.error),
        t("   ", th.border),
        t(review.branch, th.secondary),
        t(" · " <> review.behind, th.border)
      ])

    files =
      review.files
      |> Enum.with_index()
      |> Enum.map(fn {file, index} -> row(file, index == selected, th) end)

    hunk_header =
      line([
        t(Theme.glyph(:delta) <> " ", th.primary, nil, th.emphasis),
        t(current.path, th.text),
        t("  " <> plus(current.adds) <> " ", th.success),
        t(minus(current.dels), th.error),
        t("   " <> current.hunk_note <> " · j k to move", th.border)
      ])

    hunks =
      Enum.map(current.hunk, fn {kind, text} ->
        indent(2, [t("│ ", th.border), t(marker(kind), color(kind, th)), t(text, body_color(kind, th))])
      end)

    tail = [
      line([
        t(Theme.glyph(:attention) <> " ", th.warning, nil, th.emphasis),
        t(review.warning, th.muted)
      ]),
      line([
        t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis),
        t(review.tests, th.muted)
      ]),
      line([
        t("↑↓ file", th.border),
        t(" · ", th.border),
        t("j k hunk", th.border),
        t(" · ", th.border),
        t("enter open in codex", th.border),
        t(" · ", th.border),
        t("u revert hunk", th.border),
        t(" · ", th.border),
        t("c commit", th.border)
      ]),
      line([t("nothing here is committed until you say so", th.border)])
    ]

    [head, blank()] ++ files ++ [blank(), hunk_header] ++ hunks ++ [blank()] ++ tail
  end

  defp row(file, selected?, th) do
    bg = if selected?, do: th.surface, else: nil

    line([
      t(Theme.glyph(file.state) <> " ", Theme.state_color(th, file.state), bg, th.emphasis),
      t(String.pad_trailing(clip(file.path, @path - 2), @path - 1),
        if(selected?, do: th.text, else: th.muted), bg),
      t(String.pad_leading(plus(file.adds), 5) <> " ", th.success, bg),
      t(String.pad_leading(minus(file.dels), 5) <> "  ", th.error, bg),
      t(String.pad_leading(file.tests, @tests), tests_color(file.test_state, th), bg)
    ])
  end

  defp plus(nil), do: "—"
  defp plus(n), do: "+#{n}"

  defp minus(nil), do: "—"
  defp minus(n), do: "-#{n}"

  defp tests_color(:pass, th), do: th.success
  defp tests_color(:untested, th), do: th.warning
  defp tests_color(_other, th), do: th.border

  defp marker(:add), do: "+ "
  defp marker(:del), do: "- "
  defp marker(_context), do: "  "

  defp color(:add, th), do: th.success
  defp color(:del, th), do: th.error
  defp color(_context, th), do: th.border

  defp body_color(:add, th), do: th.text
  defp body_color(:del, th), do: th.muted
  defp body_color(_context, th), do: th.muted
end
