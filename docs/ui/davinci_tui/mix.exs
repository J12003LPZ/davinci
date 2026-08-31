defmodule Davinci.MixProject do
  use Mix.Project

  def project do
    [
      app: :davinci_tui,
      version: "0.1.0",
      elixir: "~> 1.12",
      start_permanent: Mix.env() == :prod,
      deps: deps()
    ]
  end

  def application do
    [extra_applications: [:logger]]
  end

  defp deps do
    [{:ratatouille, "~> 0.5"}]
  end
end
