# rustv design

One design language for the TV home screen (`crates/shell/ui/`) and the web
remote (`crates/tvd/web/`). Read this before changing either.

## Scene

A living room in the evening. The TV is across the room; the phone is in a
hand, in low light. That is why both surfaces are dark, and why everything on
the TV is large and high in contrast. It is a tool people operate, so known
controls and layouts come first and the look stays in the details.

## World

Living-room hi-fi in anodized metal and graphite: matte dark surfaces, one
metal, nothing that glows. The world lends type, palette and one signature
move; it never supplies the layout or the controls.

**Signature move: the lit plate.** Whatever is selected or primary is a solid,
flat plate of the metal with dark ink, like the illuminated active input on a
receiver. It marks:

- the focused app tile and the focused settings row on the TV
- OK on the D-pad, Send, Connect, Done, the PIN's OK
- the active tab, a selected chip, a switch that is on, the active output's icon
- the sleep timer button while a timer runs, the app running on the TV (a short bar in the dock)

Everything else is graphite. If something new needs attention, decide whether
it is selected or primary; if neither, it stays graphite.

## Palette

tvd works the palette out from three colors per finish (`crates/tvd/src/theme.rs`)
and sends it to both surfaces as the `theme` message. Four finishes:

| finish | plate | pressed | ink | background | surface | surface-2 | surface-3 | text | muted |
|---|---|---|---|---|---|---|---|---|---|
| silver (default) | #c9ced4 | #eceff2 | #0b0d0f | #0e1013 | #1d1f22 | #242629 | #2e3032 | #f2f3f3 | #9d9e9f |
| titanium | #a8b3be | #d4dce4 | #0a0c0e | #0c0f12 | #1b1e21 | #222528 | #2b2e31 | #f0f1f2 | #9a9c9e |
| champagne | #d5c4a1 | #efe5cf | #0d0c0a | #100f0d | #1f1e1b | #262522 | #302e2c | #f3f2f0 | #9e9d9a |
| gunmetal | #8e959d | #bcc2c8 | #090a0a | #0b0c0d | #191a1b | #202122 | #2a2b2c | #efeff0 | #98999a |

- `plate` is `accent`, `pressed` is `accent_2`, `ink` is `on_accent`.
- Surfaces carry 2.5% of the metal, so champagne reads warm and titanium cool.
- Ink is dark on a light metal and white on a dark custom accent, chosen by luminance.
- Contrast floors, enforced by a test: text ≥ 7:1 and muted ≥ 4.5:1 on the
  background and every surface; ink ≥ 4.5:1 on the plate and the pressed plate.
- State colors only where they mean something: ok `#7fd1a4` (connected), warn
  `#e3b866` (connecting), danger `#f2797f` (muted, close app, errors).
- Older preset names in a config (indigo, ocean, sunset, forest, rose) load as
  silver, graphite as gunmetal.

## Settings on the TV

The TV's settings (`crates/shell/ui/settings.slint`, list in `crates/shell/src/menu.rs`)
are one list, not panes: up and down move, left and right change a choice,
OK switches a toggle or cycles a choice, Back closes. Section titles are
quiet muted text, rows sit on the background, and the focused row is the lit
plate; its switch swaps colors so it stays visible on the plate. A row that
depends on a switch that is off is dimmed and skipped. What the focused row
does is shown in the column to the right of the list, not under every row.

## Over apps: keyboard, level bar, dimmer

`rustv-osd` (`crates/shell/src/bin/`) draws what tvd describes on sway's
overlay layer, over any app, and never takes focus:

- The on-screen keyboard (`crates/tvd/src/keyboard.rs`): a graphite panel
  rising from the bottom edge (top corners rounded), a recessed line with what
  was typed so far, keys in surface-2 with the focused key as the lit plate, an
  active Shift in the metal on surface-3. Special keys are words, not symbols,
  so they stay in the bundled face.
- The level bar for volume over apps and for brightness: the home screen's own
  volume overlay (surface-2 panel, muted label, metal fill on a surface-3
  track, the value in text), 230 px up from the bottom, gone 2.2 s after the
  last change.
- The dimmer draws nothing: it holds sway's gamma tables below full brightness.

## Type

Hanken Grotesk, bundled (`crates/shell/ui/fonts/`, `crates/tvd/web/fonts/`;
static instances 400 to 700 cut from the variable font, SIL OFL). Its default
digits are tabular, so the TV clock and the volume never shift.

| role | TV (at 1920×1080) | remote |
|---|---|---|
| clock | 88px, 500, tracking −2px | n/a |
| greeting / sheet titles | 52px, 600 | 20 to 22px, 600 |
| app name | 30px (600 focused, 500 otherwise) | 12px in the dock |
| body | 28px | 15 to 16px |
| hints and labels | 24px, muted | 12 to 13px, muted |

Sentence case everywhere; no uppercase tracked labels, no kicker above a heading.

## Surfaces and shape

- No borders where a step in surface tone does the job. Elevation is declared
  once: the focused TV tile and floating panels get a soft offset shadow
  (`0 22 44 #00000080` on the TV, `0 12 30 rgba(0,0,0,.5)` on the remote);
  nothing else does.
- Radii: tiles and panels 24 to 28px on the TV; 16px for remote controls,
  24px for sheets; pills only for chips, the output chip and small actions.
- Hairlines (`line`) only between settings rows, inset from the left.
- Letter tiles (apps without an icon): the initial in the metal on surface-3;
  on a focused tile, the initial in ink on a 10% ink recess.
- Browser surfaces are themed: selection, caret, focus ring (2px plate
  outline), scrollbar.

## Motion

- Focus moves with a 220ms ease-out-expo growth of the tile; the row scrolls in 240ms.
- Remote presses scale to 0.96 in 120ms; sheets rise 32px in 260ms.
- The status dot never pulses; reduced motion shortens everything to 1ms.

## Don'ts

Glows and colored halos, gradients on controls or text, glass, fake metal
reflections, cards inside cards, colored side stripes, uppercase eyebrows,
icons drawn from emoji or glyphs, and a second accent color.
