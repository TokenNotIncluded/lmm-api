# LMM Forge error page brand spec

## Product and audience

LMM Forge is a technical AI workspace. The error page should feel calm, capable, and human when a technical failure interrupts the user. It can acknowledge the failure with a little warmth, but it must not feel like a novelty 404 page, a game screen, or an enterprise alert console.

## Existing brand primitives

- Product name: LMM Forge
- Brand mark: reuse the existing React `LmmBrandMark` component from `apps/web/src/components/lmm-brand-mark.tsx` in production. Direction drafts may use a small CSS approximation only when a static HTML file cannot import the component.
- Display type: `Lora Variable` through the existing `--font-serif` token.
- UI type: `Public Sans` through the existing `--font-sans` token.
- Light paper: `#FAF9F5`
- Near black ink: `#141413`
- Card / warm neutral: `#F0EEE6`
- Muted ink: `#5E5A52`
- Cactus: `#BCD1CA`
- Clay: `#D97757`
- Oat: `#E3DACC`
- Heather: `#CBCADB`
- Coral: `#EBCECE`

## Illustration language

Use Anthropic-art principles: an opaque flat color field, large organic ivory forms, broad black gestures, simple asymmetrical composition, and one restrained accent. The supplied recovery illustration is the shared visual asset for all three directions:
`design-demos/error-page/assets/error-recovery-oat.png`.

Do not add gradients, drop shadows, glassmorphism, 3D effects, texture overlays, technical line diagrams, mascots, emoji, server racks, or extra icon collections. The art should remain legible at a glance and leave enough quiet space for the error copy.

## Copy and interaction

Use the current product copy and existing i18n keys in the actual implementation. The English reference copy is:

- Eyebrow: `System note`
- Status: `500`
- Title: `Oops! Something went wrong :')`
- Description: `We apologize for the inconvenience.`
- Follow-up: `Please try again later.`
- Note: `If this keeps happening, please report it on GitHub Issues.`
- Actions: `Go Back`, `Report an issue`, `Back to Home`

Buttons should be quiet, tactile, and clearly ordered: the recovery action first, issue reporting second, and home navigation last. Keep keyboard focus visible and preserve readable contrast in every direction.

## Avoid

The current black canvas with a thin white outlined blob and scattered controls reads unfinished and visually defensive. Avoid recreating that silhouette, using an all-black hero, or placing the art so close to the copy that it becomes a second headline.
