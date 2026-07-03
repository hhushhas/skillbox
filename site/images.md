# Image briefs for Skillbox

Design context for whatever generator you use: the site is a Vercel-inspired
dark minimal page — flat `#0a0a0a` background, `#ededed` foreground, hairline
`#262626` borders, grayscale only, no gradients or glows. Any image must look
like it belongs there: dark, geometric, quiet, essentially monochrome. If an
image arrives with a slightly different black, we can composite it on the page
background, so prefer transparent-background outputs.

## 1. Hero visual (primary ask)

Placement: right of the hero text, or below it, ~480–560px wide.
Purpose: express the pitch — skills stay boxed until you ask.

Prompt:

> Minimal isometric line-art illustration on a pure dark background (#0a0a0a).
> A single closed cube/box drawn in thin light-gray strokes (#ededed, 1.5px),
> lid slightly lifted, with one small document/file floating just above the
> opening. Surrounding the box, several faint dimmed document shapes (#666,
> lower opacity) waiting outside. No color, no gradients, no shadows, no
> texture — flat vector look, generous negative space, technical-diagram feel,
> like a Vercel or Linear marketing illustration.

Variant (more literal): the box connected by a thin dashed line to a terminal
window outline, showing "requested → delivered".

## 2. CTA / install-section backdrop (optional, use sparingly)

Placement: subtle backdrop behind the install section, very low contrast.

Prompt:

> Extremely subtle abstract pattern on #0a0a0a: sparse, thin isometric cube
> outlines scattered at large spacing, stroke color #1a1a1a (barely visible),
> flat vector, no gradients. Meant as a background texture that reads as
> near-black from a distance.

Note: skip this one if it adds any visible noise — the page works without it.

## 3. Social / Open Graph card (1200×630)

Purpose: link previews on GitHub, X, Slack.

Prompt:

> 1200×630 dark card, background #0a0a0a. Left-aligned white text (a clean
> grotesque sans, semibold): "Your skills. Loaded only when you ask." with a
> smaller gray (#a1a1a1) line under it: "skillbox — one CLI, every agent". On
> the right, the same minimal line-art open box from the hero illustration.
> Thin #262626 hairline border inset around the card. No logos other than the
> box mark, no gradients, no noise.

If the generator can't render text reliably, generate it without text and
we'll set the type in HTML/Figma over the illustration.

## What to avoid (all images)

- Color accents, gradients, glows, bokeh, 3D renders, photorealism
- Robots, brains, sparkles, circuit boards — generic "AI" clichés
- Dense compositions; every image should be mostly empty space
