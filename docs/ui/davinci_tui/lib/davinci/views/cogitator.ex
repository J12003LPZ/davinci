defmodule Davinci.Views.Cogitator do
  @moduledoc """
  The model picker, in two sizes.

  `render/1` is 1f — the small overlay that floats beside Memoria over a dimmed
  transcript. `lines/1` is 3a, the full picker `/model` opens: the same list
  with what each row costs you, and the rows you have no credential for kept on
  screen with the ramp dropped rather than hidden, so the catalog reads the same
  every time (design.md §2).

  Both state their own exits in a footer (design.md §9).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @name 30
  @window 6
  @thinking 8
  @price 14
  @credential 15

  # --- 3a: the full picker ---------------------------------------------------

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    list = Model.catalog()
    selected = rem(model.catalog_index, length(list))

    query =
      box(
        width: w,
        theme: th,
        border: th.secondary,
        body: [
          [
            t(Theme.glyph(:prompt) <> " ", th.secondary),
            t("filter models…", th.muted),
            caret(model, th)
          ]
        ]
      )

    header =
      line([
        sp(2),
        t(String.pad_trailing("PROVIDER / MODEL", @name + 1), th.border),
        t(String.pad_leading("WINDOW", @window) <> " ", th.border),
        t(String.pad_trailing("THINKING", @thinking + 1), th.border),
        t(String.pad_leading("$/Mtok", @price) <> " ", th.border),
        t("CREDENTIAL", th.border)
      ])

    rows =
      list
      |> Enum.with_index()
      |> Enum.map(fn {entry, index} -> row(entry, index == selected, th) end)

    footer = [
      line([
        t(Theme.glyph(:active) <> " ", th.border),
        t("is the ctrl+p ring", th.muted),
        t(" · ", th.border),
        t("dimmed rows have no credential", th.muted),
        t(" · ", th.border),
        t("/login xai", th.text),
        t(" adds one", th.muted)
      ]),
      line([
        t("switching keeps the transcript and re-primes the cache", th.muted)
      ]),
      line([
        t(Theme.glyph(:attention) <> " ", th.warning, nil, th.emphasis),
        t("128k of context will not fit a 32k window", th.muted)
      ]),
      line([
        t("↑↓ move", th.border),
        t(" · ", th.border),
        t("enter select", th.border),
        t(" · ", th.border),
        t("s scope to ring", th.border),
        t(" · ", th.border),
        t("esc close", th.border)
      ])
    ]

    query ++ [blank(), header] ++ rows ++ [blank()] ++ footer
  end

  defp row(entry, selected?, th) do
    absent? = entry.credential == :absent
    bg = if selected?, do: th.surface, else: nil

    state =
      cond do
        selected? -> :active
        entry.credential == :expired -> :attention
        true -> :queued
      end

    name_color =
      cond do
        selected? -> th.text
        absent? -> th.border
        true -> th.muted
      end

    detail = if absent?, do: th.border, else: th.muted

    # The ring is a second mark on the row rather than a suffix on the name:
    # trimming "anthropic / claude-sonnet" to fit a label made two rows read
    # identically.
    ring = if entry.ring, do: Theme.glyph(:active), else: " "

    left = [
      t(Theme.glyph(state) <> " ", Theme.state_color(th, state), bg, th.emphasis),
      t(clip(entry.name, @name - 2), name_color, bg),
      t(" " <> ring, th.border, bg)
    ]

    right = [
      t(String.pad_leading(entry.window, @window) <> " ", detail, bg),
      t(String.pad_trailing(entry.thinking, @thinking + 1), detail, bg),
      t(String.pad_leading(entry.price, @price) <> " ", detail, bg),
      t(String.pad_trailing(credential(entry, th), @credential), credential_color(entry, th), bg)
    ]

    line([left, sp(max(2 + @name + 1 - seg_len(left), 1), bg), right])
  end

  defp credential(%{credential: :ready} = entry, _th), do: Theme.glyph(:done) <> " " <> entry.note
  defp credential(%{credential: :expired} = entry, _th), do: Theme.glyph(:attention) <> " " <> entry.note
  defp credential(entry, _th), do: Theme.glyph(:queued) <> " " <> entry.note

  defp credential_color(%{credential: :ready}, th), do: th.success
  defp credential_color(%{credential: :expired}, th), do: th.warning
  defp credential_color(_entry, th), do: th.border

  defp caret(model, th) do
    if Model.blink?(model),
      do: t(" ", th.background, th.secondary),
      else: t(" ", th.background, th.background)
  end

  # --- 1f: the overlay beside Memoria ---------------------------------------

  def render(model) do
    th = model.theme
    inset = if Model.minimal?(model), do: 0, else: 8
    w = model.width
    inner = w - inset - 4
    list = Model.models()
    selected = rem(model.model_index, length(list))

    rows =
      list
      |> Enum.with_index()
      |> Enum.map(fn {{name, window}, index} ->
        state = if index == selected, do: :active, else: :queued
        bg = if index == selected, do: th.surface, else: nil

        left = [
          t(Theme.glyph(state) <> " ", Theme.state_color(th, state), bg, th.emphasis),
          t(name, if(index == selected, do: th.text, else: th.muted), bg)
        ]

        right = [t(window, th.border, bg)]
        [left, sp(max(inner - seg_len(left) - seg_len(right), 1), bg), right]
      end)

    footer = [
      [t("configured in ", th.muted), t("%USERPROFILE%\\.davinci\\config.toml", th.secondary)],
      [
        t("↑↓ move", th.border),
        t(" · ", th.border),
        t("enter select", th.border),
        t(" · ", th.border),
        t("esc close", th.border)
      ]
    ]

    box(
      width: w,
      indent: inset,
      theme: th,
      title: [t("COGITATOR", th.primary), t(" · ", th.border), t("MODEL", th.muted)],
      right: [t("ctrl+o", th.border)],
      body: rows ++ [[]] ++ footer
    )
  end
end
