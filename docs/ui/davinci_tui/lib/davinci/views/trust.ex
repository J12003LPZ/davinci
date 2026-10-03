defmodule Davinci.Views.Trust do
  @moduledoc """
  6a — `/trust`. The decision a project asks for the first time you open it.

  The rows are sorted by what they can do to you, not alphabetically: files that
  execute code first, files that change limits next, prose last. The composer is
  drawn disabled because nothing has been loaded yet — the screen is the only
  thing on it, and it states the choice as four keys rather than a yes/no.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @path 30
  @risk 14

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    project = Model.project_trust()

    head =
      Enum.map(
        wrap(
          "This project ships files that would change how davinci behaves. " <>
            "Nothing here has been read yet.",
          measure()
        ),
        fn row -> line([t(row, th.text)]) end
      )

    # Below 88 the description goes; what the file can do to you does not.
    detail? = model.width >= 88
    rows = Enum.map(project.files, &row(&1, detail?, th))

    decide =
      box(
        width: w,
        theme: th,
        border: th.warning,
        title: [t("DECIDE ONCE", th.warning)],
        body:
          Enum.map(
            wrap(
              "! Two of these run code you have not read. A project you cloned " <>
                "from a stranger can register a tool that reads your keys the " <>
                "first time you say hello.",
              w - 6
            ),
            fn row -> [t(row, th.text)] end
          ) ++
            [
              [],
              [
                t("decision is per path ", th.muted),
                t(project.path, th.text)
              ],
              [
                t("changeable later with ", th.muted),
                t("/trust", th.text),
                t("   --approve trusts one run", th.border)
              ],
              [],
              [
                t("[t]", th.primary),
                t(" trust, and remember", th.text),
                t("   [o]", th.primary),
                t(" this run only", th.muted)
              ],
              [
                t("[p]", th.primary),
                t(" prompts only, no code", th.muted),
                t("   [n]", th.border),
                t(" ignore them", th.border)
              ]
            ]
      )

    tail = [
      line([
        t("trusted so far ", th.muted),
        t(project.trusted, th.text),
        t(" · ignored ", th.muted),
        t(project.ignored, th.text),
        t(" · asked again when a path moves", th.border)
      ]),
      line([
        t(project.store, th.border),
        t("  paths and decisions, nothing else", th.border)
      ])
    ]

    head ++ [blank()] ++ rows ++ [blank()] ++ decide ++ [blank()] ++ tail
  end

  defp row(entry, detail?, th) do
    color = risk_color(entry.risk, th)

    line(
      [
        t(Theme.glyph(entry.state) <> " ", Theme.state_color(th, entry.state), nil, th.emphasis),
        t(String.pad_trailing(clip(entry.path, @path - 2), @path - 1), th.text)
      ] ++
        if(detail?, do: [t(String.pad_trailing(clip(entry.detail, 40), 41), th.muted)], else: []) ++
        [t(String.pad_leading(entry.risk_label, @risk), color)]
    )
  end

  defp risk_color(:executes, th), do: th.warning
  defp risk_color(:limits, th), do: th.warning
  defp risk_color(:prompt, th), do: th.muted
  defp risk_color(_harmless, th), do: th.border
end
