defmodule Davinci.Theme do
  @moduledoc """
  The only place a color literal is allowed (design.md §2).

  Tokens are resolved once at startup into whatever the terminal understands,
  so widgets just pass `theme.primary` to `color:` and never think about it.

  Copper (`primary`) carries state. Verdigris (`secondary`) carries *where
  something is* — branch, path, symbol — and never *what is happening*.
  """

  defstruct [
    :background,
    :surface,
    :surface_alt,
    :border,
    :text,
    :muted,
    :primary,
    :secondary,
    :success,
    :warning,
    :error,
    emphasis: nil,
    no_color: false,
    dimmed: false
  ]

  # xterm-256 nearest neighbours of the truecolor table in design.md §2
  @ansi %{
    background: 233,
    surface: 235,
    surface_alt: 234,
    border: 58,
    text: 187,
    muted: 102,
    primary: 173,
    secondary: 73,
    success: 108,
    warning: 179,
    error: 167
  }

  # "just drop the ramp" — no blur, no tint (design.md §2)
  @ansi_dim %{
    background: 233,
    surface: 234,
    surface_alt: 233,
    border: 236,
    text: 239,
    muted: 59,
    primary: 94,
    secondary: 66,
    success: 65,
    warning: 94,
    error: 95
  }

  # NO_COLOR: greyscale ramp, active glyphs pure white + bold (design.md §9)
  @grey %{
    background: 233,
    surface: 236,
    surface_alt: 234,
    border: 240,
    text: 254,
    muted: 248,
    primary: 231,
    secondary: 252,
    success: 231,
    warning: 231,
    error: 231
  }

  @grey_dim %{
    background: 233,
    surface: 235,
    surface_alt: 234,
    border: 237,
    text: 244,
    muted: 240,
    primary: 250,
    secondary: 242,
    success: 250,
    warning: 250,
    error: 250
  }

  @basic %{
    background: nil,
    surface: nil,
    surface_alt: nil,
    border: :black,
    text: :white,
    muted: :white,
    primary: :yellow,
    secondary: :cyan,
    success: :green,
    warning: :yellow,
    error: :red
  }

  @basic_grey %{
    background: nil,
    surface: nil,
    surface_alt: nil,
    border: :black,
    text: :white,
    muted: :white,
    primary: :white,
    secondary: :white,
    success: :white,
    warning: :white,
    error: :white
  }

  @doc "Build a theme for the negotiated color mode."
  def new(color_mode, no_color?) do
    table =
      case {color_mode, no_color?} do
        {:ansi256, false} -> @ansi
        {:ansi256, true} -> @grey
        {_, false} -> @basic
        {_, true} -> @basic_grey
      end

    struct(
      %__MODULE__{no_color: no_color?, emphasis: if(no_color?, do: [:bold], else: nil)},
      table
    )
  end

  @doc "The layer behind a modal (design.md §2)."
  def dim(%__MODULE__{dimmed: true} = theme), do: theme

  def dim(%__MODULE__{} = theme) do
    table =
      cond do
        is_atom(theme.text) and theme.no_color -> @basic_grey
        is_atom(theme.text) -> Map.put(@basic, :text, :black)
        theme.no_color -> @grey_dim
        true -> @ansi_dim
      end

    struct(%{theme | dimmed: true, emphasis: nil}, table)
  end

  # --- glyph vocabulary (design.md §4) ---------------------------------------

  @glyphs %{
    done: "✓",
    active: "◉",
    queued: "○",
    skipped: "◌",
    failed: "×",
    attention: "!",
    delta: "Δ",
    read: "↳",
    search: "⌕",
    agent: "◆",
    prompt: "›",
    user: ">",
    tick: "·",
    collapsed: "⟐"
  }

  @spinner ~w(◜ ◝ ◞ ◟)
  @pie ~w(◐ ◑ ◒ ◓)

  def glyph(name), do: Map.fetch!(@glyphs, name)

  @doc "Color that reinforces a state glyph. Never the only signal."
  def state_color(theme, state) do
    case state do
      :done -> theme.success
      :active -> theme.primary
      :queued -> theme.border
      :skipped -> theme.muted
      :failed -> theme.error
      :attention -> theme.warning
      :delta -> theme.primary
      :read -> theme.secondary
      :search -> theme.secondary
      :agent -> theme.primary
      _ -> theme.text
    end
  end

  @doc "One 4-frame spinner, 250ms per frame (design.md §8)."
  def spinner(tick, animate? \\ true)
  def spinner(_tick, false), do: glyph(:active)
  def spinner(tick, true), do: Enum.at(@spinner, rem(tick, 4))

  @doc "Proportion pie used by the narrow status bar."
  def pie(fraction) do
    Enum.at(@pie, min(trunc(fraction * 4), 3))
  end
end
