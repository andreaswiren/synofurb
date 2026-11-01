# GUI components: what exists, what the app uses, what was wrong

Source: `gpui-component` 0.8.1 (via `gpui-kit` 0.8.1) and `gpui-base` 0.8.1, read from `~/.cargo/registry/src/*/`.
Status: phase 1 (infrastructure) is built and verified; the screens of phase 2 are a first pass on these widgets and
will be reworked after the owner approves the new mockups (`design/mockups-v2/desktop/`).

## 1. Widget modules (one line each)

| Module | What it offers |
|---|---|
| `accordion` | `Accordion` / `AccordionItem`: collapsible sections, single or multiple open, bordered variant |
| `alert` | `Alert` (default/info/success/warning/error), title, icon, banner mode, closable |
| `attachment`, `bubble`, `message`, `message_scroller`, `speech`, `questionnaire` | chat / AI-assistant UI parts (unused) |
| `avatar` | `Avatar`, `AvatarGroup` |
| `badge` | `Badge`: dot / count / icon overlay on another element (update dot on Settings) |
| `breadcrumb` | `Breadcrumb` / `BreadcrumbItem` |
| `button` | `Button` (primary/danger/outline/ghost/link/text, loading, tooltip, icon), `ButtonGroup`, `DropdownButton`, `Toggle`, `ToggleGroup` (segmented) |
| `carousel`, `chart`, `plot`, `color_picker`, `calendar`/`date_picker`/`time_field`, `rating`, `diff`, `dock`, `resizable`, `inspector` | specialised widgets (unused) |
| `checkbox`, `radio` (`Radio`, `RadioGroup`), `switch`, `slider`, `select`, `combobox`, `searchable_list`, `command` | form controls and pickers |
| `clipboard` | `Clipboard`: copy button with tooltip and `on_copied` |
| `collapsible` | `Collapsible`: one animated show/hide region |
| `description_list` | `DescriptionList`: label/value grid (facts, logins) |
| `dialog` | `Dialog`, `AlertDialog`, `DialogFooter`, opened with `WindowExt::open_dialog` / `open_alert_dialog` |
| `empty` | `Empty` with media, title, description, content (empty states) |
| `form` | `Form` / `Field`: labels, descriptions, required marker, 1-n columns |
| `group_box` | `GroupBox` (normal / fill / outline): titled group; the nearest thing to a card |
| `hover_card` | `HoverCard`: content on hover (names and sizes behind a summary tag) |
| `input` | `Input` (mask toggle, cleanable, prefix/suffix, read-only), `NumberInput`, `OtpInput`, `Textarea`/editor |
| `kbd` | `Kbd`: key caps for a keystroke |
| `label`, `link`, `separator` | text label with secondary text, hyperlink, divider |
| `list`, `virtual_list`, `tree`, `table` | `List`/`ListItem`, virtualised list, tree, `Table` (static: header/body/row/cell) and `DataTable` (virtual, delegate-driven, selectable) |
| `menu` | `DropdownMenu`, `PopupMenu`, `ContextMenu`, `AppMenuBar` |
| `notification` | toasts through `WindowExt::push_notification` |
| `pagination`, `tab` (`TabBar`, `Tab`: underline / pill / segmented), `stepper` (`Stepper`, `StepperItem`), `sidebar` (`Sidebar`, `SidebarMenu`) | navigation |
| `popover` | `Popover` with trigger + content, controlled or not |
| `progress` | `Progress`, `ProgressCircle` |
| `scroll` | themed `Scrollbar`, `ScrollableElement` (`vertical_scrollbar`, `overflow_y_scrollbar`) |
| `setting` | `Settings` / `SettingPage` / `SettingGroup` / `SettingItem`: a complete settings screen |
| `sheet` | `Sheet`: drawer from an edge, `WindowExt::open_sheet` |
| `skeleton`, `spinner`, `shimmer` | loading placeholders |
| `status_bar`, `title_bar`, `toolbar` | window chrome (`TitleBar` draws client-side decorations on Linux) |
| `tag` | `Tag` (primary/secondary/danger/success/warning/info/custom colours, outline) |
| `text` | `TextView::markdown` / `html`, `Text` |
| `tooltip` | `Tooltip` (pointer only; used through `Button::tooltip` etc.) |
| `theme` | `Theme`, `ThemeConfig`, `ThemeSet` (JSON), `ThemeRegistry` |
| `gpui-kit-assets` | `IconName` (1,818 Lucide icons + 12), `icon_assets!` to embed only selected SVGs |

## 2. UI need -> widget -> where used

| UI need | Widget (module::Type) | Where used |
|---|---|---|
| Window title bar, logo, Help / Settings / menu | `TitleBar`, `Button` (icon + tooltip), `Badge` (update dot), `DropdownMenu` (`render.rs`) | every screen |
| Wizard steps | `stepper::Stepper` / `StepperItem` (horizontal; only completed steps are clickable) | `render.rs` |
| Risk tag, unit identity, "Way back" | `status_bar::StatusBar`, `Tag`, `HoverCard`, `Popover` | `render.rs` |
| Status chips (compat, risk, destroyed...) | `tag::Tag::custom` with icon + word (`kit::tag`) | everywhere |
| Inline problems | `alert::Alert` (+ `Alert::banner` for the update notice) | connect, inspect, plan, failed, done... |
| Tooltips | `Button::tooltip`, `Checkbox/Switch/Radio::tooltip`, `Tooltip` for the legend | everywhere |
| Info icon (tooltip on hover, popover on Enter, link to Help) | `Popover` + icon `Button` (`disclosure::info_tip`) | every label that has a `tip` |
| Full explanation / decisions | `Dialog`, `AlertDialog` (`disclosure::dialog`, `overlays.rs`) | confirm, plan (Keep, Slices), install (Cancel), done (Report) |
| Help centre | `Sheet` + `Input` + `TabBar` + `ListItem` + `TextView::markdown` + `Kbd` (`screens/help.rs`) | F1, `?`, title bar |
| Forms | `Form` / `Field`, `Input` (`mask_toggle`, `cleanable`, `readonly`), `Select`, `Slider`, `Checkbox`, `Switch` | connect, plan, system, bootmgr |
| Small exclusive choices | `ToggleGroup::segmented` (`kit::segmented`) | connect, plan, system, settings |
| Tables | `DataTable` (units, selectable) and `Table` (disks, arrays, slices) | inventory, inspect, recover, slices dialog |
| Facts / logins | `DescriptionList` | inspect, done, manage |
| Groups | `GroupBox::outline` | goal, plan, confirm |
| Collapsed detail | `Accordion` (bordered) | inspect, install, failed, done, about, recover, system |
| Progress | `Progress`, `Spinner` | install, update, bootmgr |
| Settings | `setting::Settings` with `SettingPage` / `SettingItem::render` | settings |
| Tabs | `TabBar` (underline) | manage, help |
| Empty states | `Empty` | inventory, manage |
| Loading rows | `Skeleton` | inventory |
| Copy buttons | `Clipboard` | system, done, manage |
| Toasts | `notification` via `WindowExt::push_notification` | copy actions |
| Key caps | `Kbd` | Help shortcuts |
| Scrollbars | `ScrollableElement::vertical_scrollbar`, themed in `theme::style_scrollbars` | page, log, settings, tables |
| Icons | `gpui_kit_assets::IconName` through `ui::icons` roles | everywhere |

## 3. Defects found in the previous UI (`crates/synofurb/src/ui`, before this rebuild) and their replacements

| Hand-made imitation (old code) | Replacement |
|---|---|
| Step rail built from `div`s with numbered circles (`render.rs`) | `Stepper` |
| Safety bar made of four chips plus a sentence (`render.rs`) | `StatusBar` with one risk `Tag`, the sentence in a `HoverCard` / `Popover` |
| `badge()`, `chip()`, `vendor_chip()` (`widgets.rs`) | `Tag::custom` (`kit::tag`) |
| `banner()` (`widgets.rs`) | `Alert` |
| `card()` with border and title (`widgets.rs`) | `GroupBox::outline` |
| `data_table()` / `Col` (`widgets.rs`) | `DataTable` / `Table` |
| `choice_card()` radio-looking cards (`widgets.rs`) | `ToggleGroup::segmented`, `GroupBox` + `Button` |
| `form_row()`, `field()`, `kv()` (`widgets.rs`) | `Form` / `Field`, `DescriptionList` |
| Log level "segmented" `div`s (`logviewer.rs`) | `ToggleGroup::segmented` |
| `divider()` | `Separator` / group borders |
| Hand-drawn status glyphs and the logo as a `server` icon | Lucide roles (`ui::icons`), the real logo (`ui::brand`) |
| Prose paragraphs under every control | `Tooltip` / `Popover` / `Dialog` / Help topics, enforced by `copy.rs` tests |
| Platform fonts (`.SystemUIFont`, DejaVu on Linux) | Inter and JetBrains Mono, bundled and registered at start-up |
| Rem size forced to 14 px by the theme (type styles shrank) | `Theme::font_size` = rem size, so the text-size setting scales everything |
| Unthemed (default) list and page scrollbars, none on the log | one themed `Scrollbar` style, applied to the Kit's base theme after every theme change |
| Mock "Update available" banner imitation | `Alert::banner` + `Button`s |

## 4. What stays hand-made, and why

| Element | Why no widget exists | Where |
|---|---|---|
| Disk -> partition -> RAID -> LVM layout diagram | no widget draws partition stacks | `diagram.rs`, inside a `GroupBox`, legend as `Tooltip`, every box carries its text |
| Log viewer rows | no log widget; rows are a virtualised `uniform_list` of mono text | `screens/logviewer.rs` (level filter is a `ToggleGroup`, search an `Input`) |
| Countdown clocks | no timer widget; shown as static text ("15 min") in this version | `screens/failed.rs` |
| Logo | brand SVG drawn with `img` (multi-colour, so not `svg()`) | `brand.rs` |
| Info icon keyboard path | the Kit's tooltip is pointer-only; the icon is a focusable `Button` whose label contains the tip and whose Enter/Space opens the same text in a popover | `disclosure.rs` |

## 5. Facts about this stack that matter

* `gpui_kit::open_window` wraps the view in `Root`; dialogs, sheets and toasts need it. `Root` sets the window rem size from `Theme::font_size`.
* The designer's `ThemeSet` JSON is loaded through `Theme::apply_config`; `theme::apply` re-applies text scale and scrollbar styles each time.
* `icon_assets!` embeds only the SVGs of `icons::role` (about 80 icons, a few tens of KB), not the 1.8k catalogue.
* Bundled fonts: Inter Regular/Medium/SemiBold/Bold (58 KB each, Latin subset, opsz 14) and JetBrains Mono Regular/Bold (65 KB each): 0.36 MB in total, OFL texts next to them (`crates/synofurb/assets/fonts/`).
