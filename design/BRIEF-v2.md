# Design brief v2 (supersedes the first mockups, which failed review by the owner)

The first round of mockups (`design/mockups/**`) is **withdrawn**. Reasons given by the owner, which are now requirements:

1. **Far too much text.** Every screen must be scannable in seconds. Explanations live behind tooltips, popovers, drawers and
   dialogs that the user opens on purpose. Only what is needed to act safely stays on the screen.
2. **GPUI components must be used properly.** Every UI element is a real `gpui-component` widget (via `gpui-kit`) unless no widget
   exists, and then the exception is listed and justified. Hand-made `div` imitations of widgets that exist (sidebar, tabs, table,
   badge/tag, alert, tooltip, popover, dialog, sheet, accordion, breadcrumb, stepper, menu, kbd, ...) are defects.
3. **Fonts:** Inter for the interface (Montserrat is allowed for the wordmark/logotype only), JetBrains Mono for code, logs, hashes.
   Bundled with the application (OFL licences included), never "whatever the system has".
4. **Icons:** Lucide (https://lucide.dev), the set that ships with `gpui-kit-assets`; inline Lucide SVGs in the web UI.
5. **Scrollbars are themed** everywhere (thin, token colours, hover state, light and dark), desktop and web.
6. **Help and About** exist in both surfaces: a Help centre (short searchable topics, shortcuts, how recovery works, how to report a
   compatibility result, link to docs) and an About page (version, licences and credits, diagnostics to copy, links). Every complex
   control also has a contextual `?`/info affordance that opens the matching help topic.
7. **Logo:** a "synofurb" logotype whose mark follows the **recycle icon** style (three chasing arrows forming a loop; "furb" is
   "refurbish"). Light and dark, mark-only and wordmark, app icon sizes, favicon.
8. **Review process:** the review artifact is the **real application** (screenshots from `synofurb gui --demo --screen <name>` and the
   web UI), not HTML imitations. The UX expert and the UI expert review those screenshots against this brief; the critics then review
   the same screenshots; nothing is shown to the owner as "ready" until both critics approve with zero blocking findings.

## Text budget (measurable)

* A screen shows at most **60 words of static prose** besides data (names, sizes, addresses, log lines). Page title: at most 4 words.
  Subtitle: at most one line (about 12 words). Labels: at most 4 words. Buttons: a verb and at most 2 more words.
* Helper text, reasons, consequences, "why is this disabled": **tooltip** (one sentence, on hover and keyboard focus), **popover**
  (a short paragraph with an action), **dialog** (the full explanation), or a **Help topic**.
* Safety-critical content stays visible but terse: the destroyed/preserved lists (names and sizes only), the unit identity, the typed phrase.
* All user-visible strings live in one copy module per surface with fields `short` / `tip` / `help`; tests enforce the word budgets.

## Disclosure patterns (use consistently)

| Need | Pattern |
|---|---|
| One sentence of explanation for a label or a disabled control | Tooltip (hover + focus), with an info icon when the control has no visible hover target |
| A paragraph with a link or action ("what is a boot flash?") | Popover opened by an info icon |
| Full explanation, a decision, or anything long | Dialog (modal) with a short title, 2-4 short paragraphs maximum, and one primary action |
| Reference material | Help centre topic (searchable, opened in a drawer/sheet or page) |
| Progress and logs | Collapsible panels, collapsed by default except the current step |

## Also required of every screen

Keyboard path, visible focus, light and dark, not colour-only status, 960x640 minimum window, no horizontal scrolling, scalable text.
