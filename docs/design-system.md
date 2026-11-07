# MyLianPC design system

The window is a desktop control panel. It should feel calm and precise: dense enough to show a
lot of live data, but with clear groups, readable type and one obvious action per area.

The visual language is implemented in:

- `crates/mylianpc-ui/src/style.css` — GTK4 tokens and component classes.
- `crates/mylianpc-ui/src/ui_kit.rs` — reusable page, card, field, pill, colour picker and preview
  widgets plus the spacing scale used by the code.
- `assets/design-tokens.json` — the same tokens as structured data for future surfaces
  (documentation site, web dashboard, slides).

## Principles

1. **Data first.** Values are the loudest element; labels are quiet and consistent.
2. **One primary action per region.** Everything else is secondary, ghost or destructive.
3. **Group by object, not by widget.** A card is one device, one curve or one settings area.
4. **Never rely on colour alone.** Status pills carry a word as well as a colour.
5. **Follow the system theme.** Semantic colours are derived from GTK theme colours, so light and
   dark both work without a second palette. The window frame is not drawn by the app: without a
   client-side titlebar the desktop supplies it — Plasma's Global Theme window decoration on KDE
   Wayland and X11, the window manager elsewhere, or GTK's default decoration on CSD-only
   compositors.

## Token layers

```
Primitive        raw values (ink, teal, violet, spacing, radius)
    ↓
Semantic         purpose (background, surface, text, accent, success, danger)
    ↓
Component        per-component (card, nav item, button, pill, input)
```

GTK4 has no CSS custom properties, so colours are `@define-color` aliases in `style.css`; the
numeric scales live in `ui_kit.rs` (`PAGE_X`, `SECTION_GAP`, `CONTROL_GAP`) and are mirrored in
`design-tokens.json`.

### Primitive colours

| Token | Value | Use |
|---|---|---|
| `pc_ink_950` | `#080b11` | deepest well |
| `pc_ink_900` | `#0c1018` | app background |
| `pc_ink_850` | `#11161f` | sidebar |
| `pc_ink_800` | `#161c27` | raised surface |
| `pc_ink_700` | `#1e2733` | hover |
| `pc_ink_500` | `#3b4759` | strong border |
| `pc_ink_300` | `#93a0b2` | muted text on dark |
| `pc_ink_100` | `#e8edf4` | primary text on dark |
| `pc_teal_500` | `#00a884` | accent / primary |
| `pc_violet_500` | `#8a6cff` | graphics / secondary accent |
| `pc_blue_500` | `#3b82f6` | connection / chart |
| `pc_green_500` | `#2ecc71` | success |
| `pc_amber_500` | `#ffb020` | warning |
| `pc_red_500` | `#ff4d5e` | danger / error |

### Semantic colours

| Token | Derivation | Use |
|---|---|---|
| `app_bg` | `@theme_bg_color` | page background |
| `sidebar_bg` | mix(theme bg, theme fg, 4.5%) | navigation |
| `surface` | mix(theme bg, theme fg, 6.5%) | cards |
| `surface_raised` | mix(theme bg, theme fg, 10%) | raised cards / toast |
| `border_color` | fg at 14% | card and input borders |
| `text_primary` | theme fg | values and headings |
| `text_muted` | fg at 62% | labels and supporting text |
| `text_faint` | fg at 42% | hints |
| `accent` | teal 500 | primary action, selection, progress |
| `accent2` | violet 500 | graphics accent |
| `success`, `warning`, `danger` | green / amber / red | status |

### Spacing

| Step | px | Use |
|---|---|---|
| `space-1` | 4 | inline icon gap |
| `space-2` | 6–8 | pill/inside controls |
| `space-3` | 10 | form rows |
| `space-4` | 12–14 | row padding |
| `space-5` | 16 | card padding / column gap |
| `space-6` | 20–24 | page margins |
| `space-7` | 32 | section separation |

### Radius

The application is mostly square. Cards, list rows, pills, swatches, previews and popovers have
a 0 px radius; buttons and inputs (entries, spin buttons, dropdowns, switches, text views) use a
subtle 4 px radius so they read as controls.

### Typography

| Role | Size / weight | Example |
|---|---|---|
| Page title | 24 px / 700 | Dashboard |
| Page subtitle | 13 px / 400, muted | Live readings… |
| Card title | 12 px / 700, muted | Processor |
| Stat value | 28 px / 700 | 43.1 °C |
| Row title | 14 px / 700 | Work |
| Body / value | 13 px | 5.4 GB used of 30.5 GB |
| Caption / hint | 11 px, faint | PNG, JPEG, GIF… |
| Mono | 13 px monospace | diagnostics |

Fonts follow the system UI font; the LCD renderer uses `DejaVu Sans` as its Linux default and
falls back automatically for old profiles that still name Segoe UI.

## Components

### Page

- Page header: title + one-sentence subtitle.
- 20 px top margin, 24 px horizontal and bottom margins.
- 16 px gap between sections.
- Content scrolls vertically only; wide content wraps or uses a flow box.

### Card

| Property | Value |
|---|---|
| Background | `surface` |
| Border | 1 px `border_color` |
| Radius | 14 px |
| Padding | 16 px |
| Title | 12 px / 700 muted |
| Gap | 8–10 px |

Cards may be flat (no fill), accent (teal tint, for selected/important content), warning or error.
Never nest more than one visual level of cards without a reason.

### Button

| Variant | Default | Hover | Disabled | Use |
|---|---|---|---|---|
| Primary | teal bg, dark text | darker teal | 50% opacity | the one action in an area |
| Secondary | 9% fg bg, border | stronger hover | 50% | alternatives |
| Ghost | transparent, muted text | hover fill | 50% | row actions, “Reload” |
| Danger | transparent, red text/border | red tint | 50% | destructive actions |
| Preset | 5% fg tile | hover fill | — | one-of-five cooling presets, selected is teal tint |

All buttons are at least 32 px high, with a 4 px radius and a visible focus border. Icon + label
buttons use a 6 px gap and a 16 px symbolic icon.

### Pill / status

| Variant | Meaning |
|---|---|
| `pill-ok` | connected / healthy / in use |
| `pill-warn` | busy / driver missing / warning |
| `pill-error` | unresponsive / error |
| `pill-muted` | not found / neutral metadata |
| `pill-accent` | active profile / capability |

Pills always contain words; colour is reinforcement only.

### Inputs

- Label above the control, 12 px semibold muted.
- Optional hint below, 11 px faint.
- 30 px minimum height, 4 px radius.
- Focus uses the teal ring.
- Switches align left and never stretch to the card width.
- Colour controls are a native picker button (swatch) plus a hex entry. The chooser is
  force-sized to 450×460 (GtkColorChooserDialog ignores `set_default_size`), the smallest measured
  size that shows the full editor without scrollbars.

### Navigation

- 222 px sidebar, 8 px horizontal padding, square selected row (surfaces stay square).
- Order: Dashboard, Lighting, LCD, Profiles, Cooling, Devices, Settings, About.
- One row = 20 px symbolic icon in a 32 px slot + label.
- Selected: teal tint, teal label, bold.
- Hover: subtle fg tint.
- Footer card: service dot + version/device count, active profile, socket path.

### Live previews

- Lighting: a strip showing the generated first frame for the selected LED count, with effect and
  brightness caption.
- Cooling: an SVG-like Cairo plot of the curve, axes 0–100 °C and 0–100 %, with the current
  temperature marker.
- LCD: the real panel preview rendered by the service, refreshed on map and every five seconds
  while visible.

## UX rules

- Pages are read-only until an explicit Apply/Save action, except the colour pickers, sliders and
  previews which only change local form state.
- Lighting shows one colour by default; **+ Add colour** adds stops up to the zone maximum, and
  trailing identical stops are trimmed so old six-field profiles do not accumulate duplicates.
- Side previews and card grids reflow to one column on narrow windows; the window is resizable
  (opening around 1150×800 and minimum roughly 880 px wide).
- The LCD preview renders the draft form via `lcd-preview-draft`; brightness and rotation are
  applied visually to the preview only, because the panel receives them as commands.
- Destructive actions ask for confirmation and use the danger variant.
- Every write reports per-device results; partial failures are shown, not hidden.
- Empty/error states explain what to do next (`sudo systemctl start mylianpc`).
- A missing sensor renders as `—`, never as a plausible-looking fake value.
- About is a standalone, last navigation entry with the application name, version, licence,
  project links and the live service summary. Service tuning and diagnostics stay in Settings.
- The service remains the only process that opens device nodes.

## Accessibility

- Minimum body text 13 px; captions 11 px only for secondary metadata.
- Muted text stays above 4.5:1 on the card surface in both themes.
- Keyboard focus is visible on buttons, entries, dropdowns and list rows.
- Status is not colour-only: every pill or dot has a text label.
- Sliders show their numeric value next to the label.
- Focused select boxes change with Up/Down (wrapping); an open menu keeps its own arrow
  navigation. The closed dropdown stays flat (no blue selected fill) and its popup uses the
  normal foreground for hovered/selected rows in both themes.
- The window opens with no control focused, so no card starts with a bright focus ring. Keyboard
  focus is drawn in the teal accent.
- Icons are decorative and always paired with text in navigation and actions.
