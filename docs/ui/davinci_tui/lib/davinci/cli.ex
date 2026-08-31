defmodule Davinci.CLI do
  @moduledoc """
  Entry point. `Ratatouille.run/2` owns the terminal for the lifetime of the app.

  `quit_events` deliberately does *not* include `q` or ctrl+c: every printable
  key belongs to the composer, and ctrl+c interrupts the run rather than the
  app (design.md §6). ctrl+d quits.
  """

  alias Ratatouille.Constants

  def main(_argv \\ []) do
    Ratatouille.run(Davinci.App,
      interval: 250,
      quit_events: [{:key, Constants.key(:ctrl_d)}]
    )
  end
end
