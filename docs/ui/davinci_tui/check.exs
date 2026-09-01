# Renders every screen against the fixtures without a terminal: catches the
# KeyErrors, MatchErrors and bad arities the compiler cannot see. Throwaway —
# not part of the app.
alias Davinci.{Model, Theme}

theme = Theme.new(:ansi256, false)

base = %Model{
  theme: theme,
  width: 100,
  height: 40,
  color_mode: :ansi256,
  animate: false,
  transcript: Model.transcript()
}

screens = [
  {:agent, nil},
  {:plan, nil},
  {:grafo, nil},
  {:memoria, nil},
  {:mensura, nil},
  {:models, nil},
  {:settings, nil},
  {:thinking, nil},
  {:login, nil},
  {:keys, nil},
  {:resume, nil},
  {:tree, nil},
  {:compact, nil},
  {:export, nil},
  {:graph_run, nil},
  {:vectors, nil},
  {:governor, nil},
  {:securitas, nil},
  {:trust, nil},
  {:officina, nil},
  {:recovery, nil},
  {:diff, nil},
  {:agent, :instrumenta},
  {:agent, :sessions},
  {:agent, :cogitator}
]

# Widths the design calls out: 80, 100, 120, 160 (design.md §7).
widths = [80, 100, 120, 160]

defmodule Check do
  @doc "Widest rendered row, walking the element tree by hand."
  def widest(%{tag: :label} = label), do: row_width(label)

  def widest(%{children: children}), do: children |> Enum.map(&widest/1) |> max()
  def widest(list) when is_list(list), do: list |> Enum.map(&widest/1) |> max()
  def widest(_leaf), do: 0

  defp max([]), do: 0
  defp max(values), do: Enum.max(values)

  defp row_width(%{children: children}) do
    Enum.reduce(children, 0, fn child, acc ->
      acc + String.length(Map.get(child.attributes, :content, ""))
    end)
  end
end

results =
  for {screen, overlay} <- screens, width <- widths do
    model = %{base | screen: screen, overlay: overlay, width: width}

    try do
      view = Davinci.App.render(model)
      bytes = view |> :erlang.term_to_binary() |> byte_size()
      {:ok, screen, overlay, width, bytes, Check.widest(view)}
    rescue
      error -> {:error, screen, overlay, width, Exception.message(error)}
    catch
      kind, value -> {:error, screen, overlay, width, "#{kind}: #{inspect(value)}"}
    end
  end

failures = Enum.filter(results, &match?({:error, _, _, _, _}, &1))

Enum.each(failures, fn {:error, screen, overlay, width, message} ->
  IO.puts("FAIL #{screen}/#{inspect(overlay)} @#{width}: #{message}")
end)

over =
  results
  |> Enum.filter(&match?({:ok, _, _, _, _, _}, &1))
  |> Enum.filter(fn {:ok, _s, _o, width, _b, widest} -> widest > width end)

Enum.each(over, fn {:ok, screen, overlay, width, _bytes, widest} ->
  IO.puts("WIDE #{screen}/#{inspect(overlay)} @#{width}: a row is #{widest} columns")
end)

IO.puts("rendered #{length(results) - length(failures)} of #{length(results)} screen/width pairs")
IO.puts(if failures == [] and over == [], do: "runtime: all clean", else: "runtime: see above")
