defmodule Davinci.Term do
  @moduledoc """
  Terminal capability negotiation.

  The design's palette is truecolor (design.md §2). termbox tops out at 256
  colors, so we ask for the 256-color output mode and report back which palette
  the theme should use:

    * `:ansi256` — colors are xterm-256 indices (nearest neighbours of the
      truecolor tokens)
    * `:basic`   — 8-color fallback, named atoms

  `NO_COLOR` and `--no-animation` are honoured here so no other module has to
  read the environment.
  """

  def enable_256_colors do
    with {:ok, mode} <- output_mode_constant(),
         {:ok, _} <- select(mode) do
      :ansi256
    else
      _ -> :basic
    end
  end

  def no_color? do
    case System.get_env("NO_COLOR") do
      nil -> false
      "" -> false
      "0" -> false
      _ -> true
    end
  end

  def animate? do
    not (Enum.member?(System.argv(), "--no-animation") or
           System.get_env("DAVINCI_NO_ANIMATION") not in [nil, "", "0"])
  end

  defp output_mode_constant do
    Enum.reduce_while([:term_output_256, :output_256, :"256"], :error, fn name, _acc ->
      try do
        {:halt, {:ok, ExTermbox.Constants.output_mode(name)}}
      rescue
        _ -> {:cont, :error}
      end
    end)
  end

  defp select(mode) do
    case ExTermbox.Bindings.select_output_mode(mode) do
      {:ok, _} = ok -> ok
      value when is_integer(value) -> {:ok, value}
      other -> other
    end
  rescue
    _ -> :error
  catch
    _, _ -> :error
  end
end
