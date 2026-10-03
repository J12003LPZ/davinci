defmodule Davinci.Views.Settings do
  @moduledoc """
  3b — `/settings`. One setting per row, its values as a ramp with the current
  one marked, and the description of the selected row directly beneath it, so
  the screen never needs a second panel to explain itself (design.md §1).

  A setting says which scope it came from — user or project — because a project
  file silently overriding a user preference is the thing that confuses people.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @label 24

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    list = Model.settings()
    selected = rem(model.settings_index, length(list))

    rows =
      list
      |> Enum.with_index()
      |> Enum.flat_map(fn {setting, index} ->
        row = row(setting, index == selected, w, th)

        if index == selected,
          do: [row | description(setting, w, th)],
          else: [row]
      end)

    footer = [
      line([
        t("user ", th.muted),
        t("%USERPROFILE%\\.davinci\\settings.json", th.text)
      ]),
      line([
        t("project ", th.muted),
        t(".davinci\\settings.json", th.secondary),
        t(" · ", th.border),
        t("overrides user", th.muted),
        t(" · ", th.border),
        t("1 key set", th.secondary)
      ]),
      line([
        t("a flag beats both scopes, for one run", th.border)
      ]),
      line([
        t("↑↓ setting", th.border),
        t(" · ", th.border),
        t("←→ value", th.border),
        t(" · ", th.border),
        t("tab scope", th.border),
        t(" · ", th.border),
        t("esc close", th.border)
      ])
    ]

    scope_bar(th) ++ [blank()] ++ rows ++ [blank()] ++ footer
  end

  defp scope_bar(th) do
    [
      line([
        t("scope ", th.muted),
        t(" user ", th.background, th.primary),
        t(" ", th.border),
        t(" project ", th.muted),
        t("   tab switches", th.border)
      ])
    ]
  end

  defp row(setting, selected?, w, th) do
    bg = if selected?, do: th.surface, else: nil
    state = if selected?, do: :active, else: :queued

    left = [
      t(Theme.glyph(state) <> " ", Theme.state_color(th, state), bg, th.emphasis),
      t(String.pad_trailing(clip(setting.label, @label), @label), if(selected?, do: th.text, else: th.muted), bg)
    ]

    values = Enum.flat_map(setting.values, &chip(&1, &1 == setting.value, bg, th))
    right = [t(scope_label(setting.scope), scope_color(setting.scope, th), bg)]
    pad = max(w - seg_len(left) - seg_len(values) - seg_len(right) - 2, 1)

    line([left, values, sp(pad, bg), right])
  end

  # The selected value is marked twice — filled, and by position — so the ramp
  # still reads under NO_COLOR (design.md §9).
  defp chip(value, true, _bg, th), do: [t(" " <> value <> " ", th.background, th.primary)]
  defp chip(value, false, bg, th), do: [t(" " <> value <> " ", th.muted, bg)]

  defp description(setting, w, th) do
    setting.description
    |> wrap(w - @label - 8)
    |> Enum.map(&indent(@label + 4, [t(&1, th.muted)]))
  end

  defp scope_label(:project), do: "project"
  defp scope_label(_), do: "user"

  defp scope_color(:project, th), do: th.secondary
  defp scope_color(_, th), do: th.border
end
