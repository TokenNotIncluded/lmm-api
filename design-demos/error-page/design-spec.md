# 500 error page direction brief

## Scope

Create three actual static HTML direction boards for the LMM Forge 500 page. These are explorations only; do not edit `apps/web/src/features/errors/` or other production files yet. Each board must be viewable at 1440x900 and remain usable down to 390px. Use the shared asset `../assets/error-recovery-oat.png` from each direction's HTML file, or an equivalent correct relative path.

The visual target is Anthropic-art: flat, opaque, warm, organic, and editorial. The supplied illustration depicts a bent signal being gently supported back into alignment. Let the image carry the emotional metaphor; the layout should provide a generous, calm reading surface around it. Use the existing fonts when available (`Public Sans` and `Lora Variable`), with sensible system fallbacks for a standalone board.

## Shared content and constraints

Every direction must contain the same information hierarchy: LMM Forge brand mark/name, `SYSTEM NOTE`, `500`, the error title, two short explanatory lines, the GitHub Issues note, and three actions. Use sentence case for the title and labels. The status may be oversized, but it must not overwhelm the title at mobile widths. The page must have visible keyboard focus, sufficient contrast, semantic landmarks, and no animation required for comprehension.

Do not use gradients, shadows, glass panels, decorative noise, thin technical diagrams, emoji, generic stock illustrations, or new third-party icon libraries. Keep the illustration unmodified as a flat image; crop it with layout and object positioning only. Do not put text over the busy part of the image. The boards should feel like three distinct compositions, not three color swaps.

## Direction A — Oat editorial spread

Use the oat field as the full page canvas. Build a restrained two-column editorial spread: a narrow left reading column with the brand and copy, and a larger right image area that lets the ivory gesture breathe. The status should be a quiet typographic anchor above the title, with a thin rule or small clay marker as the only structural accent. Buttons should sit in one compact row beneath the note. The art should be anchored right and slightly low, with ample oat margin. This is the most literary and spacious option, intended to make the failure feel considered rather than alarming.

## Direction B — Cactus recovery card

Use cactus as a broad, flat secondary field behind a centered recovery composition, with the oat illustration contained in a softly rounded but completely flat paper panel. The panel is a layout boundary, not a shadowed card: use a solid paper fill and a thin ink edge. Place the error copy on the left side of the panel and the art on the right, with `500` acting like a large editorial stamp that overlaps neither text nor image. Add a small clay “signal restored” dot or short bar only as a visual cue; do not invent extra copy. This option should feel more app-like and operational while preserving the hand-drawn warmth.

## Direction C — Coral quiet reset

Use coral as the full-bleed field and treat the page as a vertical, mobile-first poster. Put the brand at the top, the 500/title block in a focused central reading column, and the illustration below it as the emotional release. Buttons should stack cleanly on narrow screens and become a compact row on desktop. Use a paper-colored text panel or an opaque paper inset only if necessary for contrast; keep the silhouette broad and graphic. This is the most distinctive and empathetic option, with a slightly warmer “we can recover from this” tone.

## Deliverables

Write three independent files under `design-demos/error-page/directions/`: `oat-editorial.html`, `cactus-recovery.html`, and `coral-reset.html`. Include a small direction label outside the production UI so the boards can be compared, but keep it visually unobtrusive. If browser tooling is available, capture 1440x900 screenshots alongside the HTML files and verify the image path, overflow behavior, and focus state. Report changed paths and any verification performed; do not implement a final production direction without the user's selection.
