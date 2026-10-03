defmodule Davinci.Views.Studio do
  @moduledoc """
  The only box allowed mid-turn (design.md §6): a ledger of ✓ / ◉ / ○ steps with
  the active step's target appended in border color. Below 100 columns it
  collapses to one line.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def lines(model, th, w, steps) do
    if Model.narrow?(model), do: collapsed(model, th, w, steps), else: expanded(model, th, w, steps)
  end

  defp collapsed(model, th, w, steps) do
    {_state, verb, target} = active_step(steps)

    [
      line([
        t(Theme.spinner(model.tick, model.animate) <> " ", th.primary),
        t("studying ", th.muted),
        t(clip(target || verb, w - 14), th.secondary)
      ])
    ]
  end

  defp expanded(model, th, w, steps) do
    width = min(w, measure() + 6)

    body =
      Enum.map(steps, fn {state, verb, target} ->
        glyph =
          if state == :active,
            do: Theme.spinner(model.tick, model.animate),
            else: Theme.glyph(state)

        [
          t(glyph <> " ", Theme.state_color(th, state), nil, th.emphasis),
          t(verb, if(state == :queued, do: th.muted, else: th.text))
        ] ++
          if target,
            do: [t("  ", th.border), t(clip(target, width - 40), th.border)],
            else: []
      end)

    box(width: width, theme: th, title: [t("STUDIO", th.primary)], body: body)
  end

  defp active_step(steps) do
    Enum.find(steps, List.first(steps), fn {state, _, _} -> state == :active end)
  end
end
