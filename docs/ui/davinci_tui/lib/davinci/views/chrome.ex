defmodule Davinci.Views.Chrome do
  @moduledoc """
  AppShell: header, composer, status bar (design.md §6).

  Header and status bar are one line each at every width; both abbreviate rather
  than wrap. The composer is the loudest element on screen: copper rule, `›`
  prompt, blinking block caret, keybind hints below it in border color.
  """

  import Ratatouille.View
  import Davinci.Ui

  alias Davinci.{Model, Theme}

  def header(model) do
    th = model.theme
    w = model.width

    left = [
      t("D", th.primary, nil, th.emphasis),
      t(" davinci", th.text),
      t(" · ", th.border),
      t(Model.mode(model), th.primary)
    ]

    right =
      if Model.minimal?(model) do
        [t("davinci-rust", th.muted), t(" │ ", th.border), t(model.branch, th.secondary)]
      else
        [
          t(model.cwd, th.muted),
          t(" │ ", th.border),
          t(model.branch, th.secondary),
          t(" │ ", th.border),
          t(model.model_name, th.muted)
        ] ++
          if Model.codex_open?(model),
            do: [t(" │ ", th.border), t("#{model.width}×#{model.height}", th.border)],
            else: []
      end

    bar([spread(w, left, right)])
  end

  def status(model) do
    th = model.theme
    bar([spread(model.width, status_left(model, th), status_right(model, th))])
  end

  defp status_left(model, th) do
    {delta, adds, dels} = model.changes

    case model.screen do
      :grafo ->
        [
          t("grafo", th.primary),
          t(" · ", th.border),
          t(model.branch, th.secondary),
          t(" · ", th.border),
          t("impact view", th.muted)
        ]

      :memoria ->
        [t("memoria", th.primary), t(" · ", th.border), t("recall 6 of 18,402", th.muted)]

      :mensura ->
        [
          t("mensura", th.primary),
          t(" · ", th.border),
          t(model.branch, th.secondary),
          t(" · ", th.border),
          t("1 proposal", th.warning)
        ]

      :plan ->
        [
          t("plan", th.primary),
          t(" · ", th.border),
          t(model.branch, th.secondary),
          t(" · ", th.border),
          t("5 steps", th.muted)
        ]

      _ ->
        if Model.minimal?(model) do
          [t(model.branch, th.secondary), t(" · ", th.border), t("Δ#{delta}", th.primary)]
        else
          [
            t(Model.mode(model), th.primary),
            t(" · ", th.border),
            t(model.branch, th.secondary),
            t(" · ", th.border),
            t("Δ#{delta} ", th.primary),
            t("+#{adds} ", th.success),
            t("-#{dels}", th.error)
          ] ++
            if Model.codex_open?(model),
              do: [t(" · ", th.border), t("codex open", th.muted)],
              else: []
        end
    end
  end

  defp status_right(%{screen: :grafo}, th) do
    [
      t("enter open node", th.border),
      t(" · ", th.border),
      t("x expand", th.border),
      t(" · ", th.border),
      t("esc close", th.border)
    ]
  end

  defp status_right(model, th) do
    {used, cap} = Model.context(model)
    fraction = Model.context_fraction(model)

    cond do
      Model.minimal?(model) ->
        # Still a meter, never a bare number (design.md §6, §9).
        [
          t(Theme.pie(fraction), th.primary),
          t(" #{round(fraction * 100)}%", th.muted),
          t(" · ", th.border),
          t("^p", th.border)
        ]

      Model.narrow?(model) ->
        [
          t("mensura ", th.muted),
          t(Theme.pie(fraction), th.primary),
          t(" #{round(fraction * 100)}%", th.muted),
          t(" · ", th.border),
          t("^p", th.border)
        ]

      true ->
        [t("context ", th.muted)] ++
          meter(fraction, 12, th) ++
          [t(" #{round(used / 1000)}k/#{round(cap / 1000)}k", th.muted)]
    end
  end

  @doc """
  Grows with content: `:lines` renders a multi-line entry (1c), otherwise the
  single live composer line. Always returns the box plus one hint row.
  """
  def composer(model, opts \\ []) do
    th = Keyword.get(opts, :theme, model.theme)
    hint = Keyword.get(opts, :hint, :default)
    placeholder = Keyword.get(opts, :placeholder, "ask davinci…")
    inset = Keyword.get(opts, :indent, 0)
    w = model.width
    entries = Keyword.get(opts, :lines) || [model.composer]
    last = length(entries) - 1

    caret =
      if Model.blink?(model),
        do: t(" ", th.background, th.primary),
        else: t(" ", th.background, th.background)

    rows =
      entries
      |> Enum.with_index()
      |> Enum.map(fn {string, index} ->
        entry =
          if string == "",
            do: t(placeholder, th.muted),
            else: t(clip(string, w - 10), th.text)

        [t("#{Theme.glyph(:prompt)} ", th.primary), entry] ++
          if(index == last, do: [caret], else: [])
      end)

    box(width: w, indent: inset, theme: th, border: th.primary, body: rows) ++
      [hint_line(model, th, hint, inset)]
  end

  defp hint_line(model, th, hint, inset) do
    segs =
      case hint do
        :closable ->
          [t("enter run", th.border), t(" · ", th.border), t("esc close", th.border)]

        :recall ->
          [
            t("enter pin to context", th.border),
            t(" · ", th.border),
            t("f raise floor", th.border),
            t(" · ", th.border),
            t("r reindex", th.border),
            t(" · ", th.border),
            t("esc close", th.border)
          ]

        :multiline ->
          [
            t("shift+enter newline", th.border),
            t(" · ", th.border),
            t("2 lines", th.border),
            t(" · ", th.border),
            t("enter send", th.border)
          ]

        :none ->
          []

        _ ->
          if Model.minimal?(model) do
            [t("enter send · esc cancel", th.border)]
          else
            [
              t("enter send", th.border),
              t(" · ", th.border),
              t("shift+enter newline", th.border),
              t(" · ", th.border),
              t("tab complete", th.border),
              t(" · ", th.border),
              t("esc cancel", th.border)
            ]
          end
      end

    indent(inset + 2, segs)
  end
end
