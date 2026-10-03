defmodule Davinci.Views.Officina do
  @moduledoc """
  6b — `/reload`. What is loaded, what failed to load, and what it costs every
  turn.

  The reload result is written as ordinary tool lines (design.md §3): the elbow,
  the state glyph, what it did, then how long it took. A failed extension keeps
  its error and says which of its tools are missing as a result, rather than
  disappearing quietly.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    workshop = Model.workshop()

    echo = line([t(Theme.glyph(:user) <> " ", th.primary), t("/reload", th.muted)])

    result =
      Enum.flat_map(workshop.reload, fn
        {state, text, duration, nil} ->
          [reload_line(state, text, duration, th)]

        {state, text, duration, detail} ->
          [reload_line(state, text, duration, th)] ++
            Enum.map(wrap(detail, measure() - 6), &indent(6, [t(&1, th.error)]))
      end)

    native =
      box(
        width: w,
        theme: th,
        title: [t("NATIVE", th.primary), t(" · ", th.border), t("RUST, ALWAYS ON", th.muted)],
        right: [t("0ms", th.border)],
        body: Enum.map(workshop.native, &extension_row(&1, th))
      )

    javascript =
      box(
        width: w,
        theme: th,
        title: [t("JAVASCRIPT", th.primary), t(" · ", th.border), t("NODE SUBPROCESS", th.muted)],
        right: [t(workshop.node, th.border)],
        body: Enum.map(workshop.javascript, &extension_row(&1, th))
      )

    schema =
      [
        line([
          t("what every turn carries", th.text),
          t("   ", th.border),
          t(workshop.schema, th.warning),
          t(" of the window is tool schema", th.muted)
        ])
      ] ++
        Enum.map(workshop.tools, fn {label, count, fraction, note} ->
          line([
            t(String.pad_trailing(label, 16), th.muted),
            t(String.pad_leading(count, 4) <> "  ", th.text)
          ] ++ meter(fraction, 14, th, th.secondary) ++ [t("  " <> note, th.border)])
        end)

    footer = [
      line([
        t("-nt disables all tools", th.border),
        t(" · ", th.border),
        t("-t read,grep,ls keeps three", th.border),
        t(" · ", th.border),
        t("-xt bash drops one", th.border)
      ]),
      line([
        t("/reload keeps the session and the transcript", th.muted),
        t(" · ", th.border),
        t("e show the error", th.border),
        t(" · ", th.border),
        t("esc close", th.border)
      ])
    ]

    [echo, blank()] ++
      result ++
      [blank()] ++ native ++ [blank()] ++ javascript ++ [blank()] ++ schema ++ [blank()] ++ footer
  end

  defp reload_line(state, text, duration, th) do
    indent(2, [
      t("⎿ ", th.border),
      t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
      t(text, if(state == :failed, do: th.text, else: th.muted)),
      t("   " <> duration, th.border)
    ])
  end

  defp extension_row({state, name, detail}, th) do
    [
      t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
      t(String.pad_trailing(name, 20), if(state == :failed, do: th.border, else: th.muted)),
      t(detail, th.border)
    ]
  end
end
