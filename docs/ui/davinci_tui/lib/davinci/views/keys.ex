defmodule Davinci.Views.Keys do
  @moduledoc """
  3e — `/hotkeys`. The whole keymap, grouped by the surface a key belongs to.

  The mockup sets this in two columns; on a character grid at 88 columns one
  column per surface reads better and keeps every row inside the measure
  (design.md §3), so the groups stack and the sheet scrolls with ↑↓ instead.
  The point the screen has to make survives either way: a key means one thing
  per surface, and ctrl+d means three different things depending on what has
  the keyboard.

  Unlike the transcript, this sheet is windowed from the top and says how much
  is below it — dropping the first rows of a reference sheet to fit the window
  would hide exactly what someone opened it to read.
  """

  import Davinci.Ui

  alias Davinci.Model

  @key 18

  def lines(model, rows \\ nil) do
    th = model.theme
    w = min(model.width, measure() + 14)

    body =
      Model.keymap()
      |> Enum.flat_map(fn {title, note, bindings} ->
        [section(title, note, w, th)] ++ Enum.map(bindings, &row(&1, th)) ++ [blank()]
      end)

    footer = [
      line([t("a key means one thing per surface", th.muted)]),
      line([t("ctrl+d quits here, deletes in the session list", th.border)]),
      line([
        t("rebind in ", th.muted),
        t("%USERPROFILE%\\.davinci\\keybindings.json", th.secondary)
      ]),
      line([t("esc close", th.border)])
    ]

    window(body, footer, model, th, rows)
  end

  defp window(body, footer, _model, _th, nil), do: body ++ footer

  defp window(body, footer, model, th, rows) do
    room = max(rows - length(footer) - 1, 4)

    if length(body) <= room do
      body ++ footer
    else
      offset = min(model.keys_offset, length(body) - room)
      shown = body |> Enum.drop(offset) |> Enum.take(room)
      below = length(body) - offset - room

      shown ++ [scroll_note(offset, below, th)] ++ footer
    end
  end

  defp scroll_note(offset, below, th) do
    above = if offset > 0, do: "#{offset} above", else: nil
    more = if below > 0, do: "#{below} below", else: nil

    counts =
      [above, more]
      |> Enum.reject(&is_nil/1)
      |> Enum.join(" · ")

    line([t("↑↓ scrolls", th.border), t("  " <> counts, th.muted)])
  end

  defp section(title, note, w, th) do
    right = if note == "", do: [], else: [t(note, th.border)]
    spread(w, [t(title, th.primary)], right)
  end

  defp row({key, description}, th) do
    line([
      sp(2),
      t(String.pad_trailing(key, @key), th.text),
      t(description, if(destructive?(key), do: th.warning, else: th.muted))
    ])
  end

  # The destructive bindings are marked wherever they are listed, so the sheet
  # never reads as a flat list of equals.
  defp destructive?(key), do: key in ["ctrl+d", "ctrl+backspace"]
end
