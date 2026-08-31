defmodule Davinci.Views.Cogitator do
  @moduledoc """
  1f — model / provider picker. Overlay over a dimmed transcript; states its own
  exits in its footer (design.md §9).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

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
