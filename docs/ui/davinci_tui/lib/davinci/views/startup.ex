defmodule Davinci.Views.Startup do
  @moduledoc """
  1a — startup and empty state. The identity mark appears here and in the empty
  state, nowhere else (design.md §10). The smile is the only copper stroke.
  Below 100 columns the emblem is dropped entirely (design.md §7).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @emblem [
    "      ·───────·",
    "    ╱           ╲",
    "   ╱  ╭───────╮  ╲",
    "  ╱  ╱  ·   ·  ╲  ╲",
    " │  │     ╷     │  │",
    " │   ╲  ╰───╯  ╱   │",
    "  ╲   ╰───────╯   ╱",
    "   ╲             ╱",
    "    ╲     ╷     ╱",
    "   ╱─────────────╲",
    "  │               │",
    " ╱     ╰──┬──╯     ╲",
    "·─────────────────────·"
  ]

  # the smile: line 6, the ╰───╯ run
  @smile_row 5
  @smile "╰───╯"

  def lines(model) do
    th = model.theme
    w = model.width

    emblem = if Model.decoration?(model), do: emblem(th, w) ++ [blank()], else: []

    emblem ++
      [
        center(w, [t("D A V I N C I", th.text, nil, th.emphasis)]),
        center(w, [t("macchina dell'intelletto", th.muted)]),
        blank(),
        center(w, [t(model.cwd, th.secondary)]),
        center(w, [
          t(model.branch, th.secondary),
          t(" · ", th.border),
          t("rust", th.muted),
          t(" · ", th.border),
          t("11 crates", th.muted)
        ]),
        center(w, [
          t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis),
          t("session restored", th.muted),
          t(" · ", th.border),
          t("memoria intacta", th.muted)
        ]),
        blank(),
        hair_rule(min(w, 62), th) |> recenter(w, min(w, 62)),
        blank(),
        center(w, [t("A machine for thought, built in Rust.", th.text)])
      ] ++
      if Model.decoration?(model),
        do: [blank(), center(w, [t("proportio humana", th.border)])],
        else: []
  end

  defp emblem(th, w) do
    @emblem
    |> Enum.with_index()
    |> Enum.map(fn {row, index} ->
      center(w, emblem_row(row, index, th))
    end)
  end

  defp emblem_row(row, @smile_row, th) do
    [head, tail] = String.split(row, @smile, parts: 2)
    [t(head, th.muted), t(@smile, th.primary, nil, th.emphasis), t(tail, th.muted)]
  end

  defp emblem_row(row, _index, th), do: [t(row, th.muted)]

  defp recenter(%{tag: :label} = label, width, inner) do
    %{label | children: [sp(max(div(width - inner, 2), 0)) | label.children]}
  end
end
