defmodule Davinci.App do
  @moduledoc """
  The Elm-architecture app: one instrument at a time, summoned by key, dismissed
  with esc (design.md §1).

  Layout is assembled as a flat list of rows so the composer can be anchored to
  the bottom of the window at any height, and so an overlay can dim the
  transcript behind it by simply rendering it with the dimmed ramp.
  """

  @behaviour Ratatouille.App

  import Ratatouille.View

  alias Davinci.{Model, Theme, Ui}
  alias Davinci.Views.{Chrome, Cogitator, Codex, Disegno, Grafo, Instrumenta, Memoria,
                       Mensura, Startup, Transcript}
  alias Ratatouille.Constants
  alias Ratatouille.Runtime.Subscription

  @esc Constants.key(:esc)
  @enter Constants.key(:enter)
  @space Constants.key(:space)
  @backspace Constants.key(:backspace)
  @backspace2 Constants.key(:backspace2)
  @up Constants.key(:arrow_up)
  @down Constants.key(:arrow_down)
  @ctrl_c Constants.key(:ctrl_c)
  @ctrl_e Constants.key(:ctrl_e)
  @ctrl_g Constants.key(:ctrl_g)
  @ctrl_l Constants.key(:ctrl_l)
  @ctrl_o Constants.key(:ctrl_o)
  @ctrl_p Constants.key(:ctrl_p)
  @ctrl_r Constants.key(:ctrl_r)
  @ctrl_s Constants.key(:ctrl_s)
  @ctrl_u Constants.key(:ctrl_u)
  @resize Constants.event_type(:resize)

  @impl true
  def init(context), do: Model.new(context)

  @impl true
  def subscribe(_model), do: Subscription.interval(250, :tick)

  @impl true
  def update(model, msg) do
    case msg do
      :tick ->
        %{model | tick: model.tick + 1}

      {:event, %{type: @resize, w: w, h: h}} when w > 0 and h > 0 ->
        %{model | width: w, height: h}

      {:event, %{key: @ctrl_c}} ->
        Model.interrupt(model)

      {:event, %{key: @esc}} ->
        %{model | screen: :agent, overlay: nil}

      {:event, %{key: @ctrl_p}} ->
        toggle_overlay(model, :instrumenta)

      {:event, %{key: @ctrl_s}} ->
        toggle_overlay(model, :sessions)

      {:event, %{key: @ctrl_o}} ->
        toggle_overlay(model, :cogitator)

      {:event, %{key: @ctrl_l}} ->
        toggle_screen(model, :plan)

      {:event, %{key: @ctrl_g}} ->
        toggle_screen(model, :grafo)

      {:event, %{key: @ctrl_r}} ->
        toggle_screen(model, :memoria)

      {:event, %{key: @ctrl_u}} ->
        toggle_screen(model, :mensura)

      {:event, %{key: @ctrl_e}} ->
        %{model | codex: not model.codex, overlay: nil, screen: :agent}

      {:event, %{key: @up}} ->
        move(model, -1)

      {:event, %{key: @down}} ->
        move(model, +1)

      {:event, %{key: @enter}} ->
        if model.overlay, do: %{model | overlay: nil}, else: Model.submit(model)

      {:event, %{key: key}} when key in [@backspace, @backspace2] ->
        erase(model)

      {:event, %{key: @space}} ->
        insert(model, " ")

      {:event, %{ch: ch}} when ch > 0 ->
        insert(model, <<ch::utf8>>)

      _ ->
        model
    end
  end

  defp toggle_overlay(model, name) do
    if model.overlay == name,
      do: %{model | overlay: nil},
      else: %{model | overlay: name, screen: :agent}
  end

  defp toggle_screen(model, name) do
    if model.screen == name,
      do: %{model | screen: :agent},
      else: %{model | screen: name, overlay: nil}
  end

  defp insert(%{overlay: :instrumenta} = model, string) do
    %{model | query: model.query <> string, palette_index: 0}
  end

  defp insert(model, string), do: Model.type(model, string)

  defp erase(%{overlay: :instrumenta} = model) do
    %{model | query: String.slice(model.query, 0, max(String.length(model.query) - 1, 0))}
  end

  defp erase(model), do: Model.backspace(model)

  defp move(model, delta) do
    cond do
      model.overlay == :instrumenta ->
        %{model | palette_index: wrap(model.palette_index + delta, length(Model.filtered_corpus(model)))}

      model.overlay == :sessions ->
        %{model | session_index: wrap(model.session_index + delta, length(Model.sessions()))}

      model.overlay == :cogitator ->
        %{model | model_index: wrap(model.model_index + delta, length(Model.models()))}

      model.screen == :memoria ->
        %{model | recall_index: wrap(model.recall_index + delta, length(Model.recall()))}

      true ->
        model
    end
  end

  defp wrap(_index, 0), do: 0
  defp wrap(index, count), do: Integer.mod(index, count)

  # --- render ---------------------------------------------------------------

  @impl true
  def render(model) do
    rows = max(model.height - 2, 8)

    view(
      [top_bar: Chrome.header(model), bottom_bar: Chrome.status(model)],
      body(model, rows)
    )
  end

  defp body(model, rows) do
    composer = composer(model)
    content_rows = max(rows - length(composer), 1)

    content =
      if Model.codex_open?(model) do
        [Codex.render(model, content_rows)]
      else
        model
        |> content()
        |> Ui.tail(content_rows)
        |> Ui.pad_to(content_rows)
      end

    content ++ composer
  end

  defp content(model) do
    case {model.screen, model.overlay} do
      {:plan, _} ->
        Disegno.lines(model)

      {:grafo, _} ->
        Grafo.lines(model)

      {:memoria, _} ->
        Memoria.recall(model)

      {:mensura, _} ->
        Mensura.lines(model)

      {_, :instrumenta} ->
        dimmed(model) ++ [Ui.blank()] ++ Instrumenta.lines(model)

      {_, :sessions} ->
        dimmed(model) ++ [Ui.blank()] ++ Memoria.sessions(model)

      {_, :cogitator} ->
        dimmed(model) ++ [Ui.blank()] ++ Cogitator.render(model)

      _ ->
        transcript(model)
    end
  end

  defp transcript(model) do
    entries = entries(model)
    if entries == [], do: Startup.lines(model), else: Transcript.lines(model, entries: entries)
  end

  # Behind a modal the whole ramp drops; never blur, never tint (design.md §2).
  defp dimmed(model) do
    model
    |> entries()
    |> then(&Transcript.lines(model, entries: &1, theme: Theme.dim(model.theme)))
    |> Ui.tail(10)
  end

  # At 80 columns the fixture transcript is the narrow one; anything the user
  # has typed always wins.
  defp entries(model) do
    if Model.narrow?(model) and model.transcript == Model.transcript(),
      do: Model.narrow_transcript(),
      else: model.transcript
  end

  defp composer(model) do
    case {model.screen, model.overlay} do
      {:plan, _} ->
        Chrome.composer(model,
          hint: :multiline,
          lines: [
            "keep step IV, but generate the fixtures from the",
            "existing TS golden files under tests\\golden\\"
          ]
        )

      {:memoria, _} ->
        Chrome.composer(model, hint: :recall)

      {:grafo, _} ->
        Chrome.composer(model,
          placeholder: "/graph path davinci-cli::main → store::write",
          hint: :closable
        )

      {:mensura, _} ->
        Chrome.composer(model, placeholder: "/mensura policy frugal", hint: :closable)

      {_, nil} ->
        Chrome.composer(model)

      _ ->
        # Dimmed with the transcript: the palette owns the keyboard.
        Chrome.composer(model, theme: Theme.dim(model.theme), hint: :none)
    end
  end
end
