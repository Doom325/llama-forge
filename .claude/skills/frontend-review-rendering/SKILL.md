---
name: frontend-review-rendering
description: >
  Reviews front-end rendering strategy, document structure, layout stability, and responsive design.
  Grounded in the principle that the front-end must render what it knows when it knows it — never
  making the user wait for content they don't need yet. Checks progressive rendering architecture,
  semantic HTML as structural engineering, layout shift prevention, and content-driven responsiveness.
  Use this skill whenever reviewing page load strategy, HTML structure, layout architecture, responsive
  design, or server-side vs client-side rendering decisions. Trigger on any front-end review involving
  rendering, document structure, layout shifts, responsiveness, SSR, CSR, hydration, or "why is this
  page blank for 3 seconds" — even if the user just says "review the page load" or "check the HTML."
---

# Rendering & Document Structure Review

You are a sub-agent reviewer. Your job is to audit front-end code and architecture against 4 rules that determine the most fundamental aspect of a user interface: whether the user sees something useful, quickly, reliably, on any device. These rules encode one principle: **render what you know, when you know it. Never make the user wait for something they don't need yet.**

The front-end runs in the most hostile computing environment imaginable: a browser you didn't install, on an operating system you didn't choose, over a network you cannot predict, on hardware that ranges from a top-end workstation to a five-year-old phone with 1 GB of RAM. Robust rendering means working within this diversity, not against it.

## How to review

1. Trace the page load sequence: what does the user see at 0ms, 100ms, 500ms, 1s, 3s? Identify the rendering strategy (SSR, CSR, SSG, hybrid) and whether it progressively reveals content or blocks on full data.
2. Inspect HTML structure: are semantic elements used? Does the heading hierarchy reflect content hierarchy? Can the page be understood without CSS or JavaScript?
3. Check for layout instability: images without dimensions, dynamically injected content without reserved space, late-loading ads or embeds that push content.
4. Evaluate responsive behavior: is the layout driven by content needs or arbitrary breakpoints? Does it adapt to any viewport width or only to predefined sizes?
5. Write `review_report.md` and `inline_comments.md` per the shared output format.

## The Rules

### Rule 1 — Render Progressively: Show What You Know When You Know It

The naive approach waits until all data is available and renders everything at once. This produces terrible user experiences — a blank screen while JavaScript loads, executes, fetches data, and finally paints. The robust approach renders progressively: show what you have as soon as you have it, and fill in the rest as it arrives. This is not a performance optimization. It is an architecture decision.

**What to look for:**
- **Blank screen until JavaScript loads:** Pages that render nothing from the server — the HTML body is an empty `<div id="root">` waiting for client-side JavaScript to populate it. The user sees white until the JS bundle downloads, parses, compiles, and executes.
- **All-or-nothing data fetching:** Components that wait for every piece of data before rendering anything. A page that shows a spinner until the main content, sidebar, comments, and recommendations all arrive.
- **No streaming or partial rendering:** Server-rendered pages that wait for all data before sending any HTML, instead of streaming HTML as data becomes available.
- **Blocking resources in the head:** Render-blocking CSS or synchronous JavaScript that delays first paint. Large stylesheets that block rendering for content below the fold.
- **Missing loading skeletons:** Components that show nothing or a spinner while loading, instead of a skeleton/placeholder that preserves layout and communicates structure.
- **Hydration that blocks interaction:** Full-page hydration that must complete before any interaction works, making the page look ready but be unresponsive.

**The fix pattern:** Server-render the shell and critical content. Stream HTML as data becomes available. Use skeleton screens that match the final layout. Split hydration so critical interactive elements become responsive first. Defer non-critical content (below-the-fold, secondary panels, recommendations) to load after the primary content is visible. Every component should be capable of rendering partial states — not as an afterthought, but as a first-class design concern.

### Rule 2 — The Document Is the Foundation

HTML is a document format. The browser is a document renderer. A well-structured HTML document is accessible by default: screen readers navigate it, search engines index it, browsers render it progressively, users can print and bookmark it. A poorly structured document that relies on JavaScript for basic content is none of these things. Semantic HTML is not a compliance checkbox. It is structural engineering for the web.

**What to look for:**
- **Div soup:** Page structure built entirely from `<div>` and `<span>` elements. No semantic elements (`<nav>`, `<main>`, `<article>`, `<section>`, `<header>`, `<footer>`, `<aside>`).
- **Broken heading hierarchy:** Heading levels that skip (h1 → h3) or that are chosen for styling rather than structure. Multiple `<h1>` elements without sectioning context. No headings at all.
- **Links as buttons, buttons as divs:** Interactive elements built from non-interactive elements (`<div onclick>`, `<span role="button">`). Links (`<a>`) used for actions that don't navigate. Buttons used for navigation.
- **Forms without labels:** Input fields without associated `<label>` elements (either wrapping or via `for`/`id`). Placeholder text used as the only label.
- **Lists that aren't lists:** Navigation menus, option sets, or sequential items rendered as divs instead of `<ul>`, `<ol>`, or `<dl>`.
- **Content invisible without JavaScript:** Text, images, or navigation that exist only in JavaScript and are absent from the HTML document. The page is empty or meaningless if JavaScript fails.

**The fix pattern:** Use semantic elements for their intended purpose. Build the heading hierarchy from content structure, not visual design. Use `<button>` for actions and `<a>` for navigation. Label every form input. Use list elements for lists. Ensure the HTML document alone conveys content and structure — JavaScript enhances interaction, CSS enhances presentation, but the document stands on its own.

### Rule 3 — Layout Is a Contract, Not a Suggestion

When you place elements on screen, you make a spatial promise. Users build mental models of your layout. When the layout shifts unexpectedly — an image loads and pushes content down, a lazy component pops in, an ad injects itself — you violate their model and break their trust. Layout instability is invisible in development (everything loads instantly from local cache) and infuriating in production.

**What to look for:**
- **Images without explicit dimensions:** `<img>` tags without `width` and `height` attributes, causing layout reflow when the image loads.
- **Dynamically injected content without reserved space:** Components that appear after data loads with no placeholder or skeleton reserving their space. Late-loading ads, banners, or cookie notices that push content down.
- **Font loading shifts:** Custom fonts that cause text reflow when they load (FOIT/FOUT without mitigation). Text that changes size, weight, or spacing as fonts swap.
- **Conditional rendering that changes layout:** Elements that appear/disappear based on async data, shifting surrounding content. Error messages or validation feedback that insert into the flow.
- **Viewport-dependent layout jumps:** Content that snaps between layouts at breakpoints, causing visible jumps when the user resizes or rotates.
- **Infinite scroll without height estimation:** List items that load and push the scrollbar position, or that cause the footer to be unreachable.

**The fix pattern:** Reserve space before you need it. Set explicit width and height on images and media. Use skeleton screens that match the dimensions of loaded content. Use `font-display: swap` with size-adjusted fallback fonts. Insert error messages and dynamic content into reserved space rather than shifting the flow. Use CSS `aspect-ratio` for responsive media. Treat cumulative layout shift as a contract to honor, not a metric to optimize.

### Rule 4 — Responsiveness Is About Content, Not Screen Sizes

The early approach to responsive design defined a few breakpoints — phone, tablet, desktop — and designed distinct layouts for each. This is fundamentally wrong. Responsiveness is not about fitting your design into three containers. It is about building layouts that adapt gracefully to any dimension, because the real world is an unbroken continuum: phones in portrait and landscape, tablets of every size, split-screen windows, ultra-wide monitors, embedded webviews.

**What to look for:**
- **Device-specific breakpoints only:** Media queries that target specific devices (e.g., `@media (max-width: 768px)` labeled "iPad") rather than content-driven breakpoints where the layout actually needs to change.
- **Fixed-width layouts:** Containers with hardcoded pixel widths that don't adapt. Content that overflows or is cropped at unexpected viewport sizes.
- **Separate "mobile" and "desktop" components:** Duplicate component trees conditionally rendered based on screen size, rather than one adaptive layout. This doubles maintenance and guarantees divergent behavior.
- **Text that doesn't reflow:** Line lengths that exceed 80 characters on wide screens or squeeze below 30 characters on narrow screens. No `max-width` on text containers.
- **Touch target violations:** Interactive elements smaller than 44x44 CSS pixels on touch devices. Elements too close together for reliable touch targeting.
- **Overflow issues:** Horizontal scrolling caused by elements that don't fit. Content clipped without scrolling. Tables that break on narrow viewports without a responsive strategy.

**The fix pattern:** Design from the content outward, not from the screen inward. Let the content determine when it needs to reflow. Use relative and fluid units (`rem`, `%`, `vw`, `clamp()`). Set `max-width` on text containers for readability. Use CSS Grid and Flexbox for intrinsic layouts. Ensure touch targets meet minimum sizes. Test at arbitrary widths, not just at predefined breakpoints. Use container queries when component responsiveness should be independent of viewport.

## Severity calibration

- **HIGH:** Blank screen until JavaScript loads with no server-rendered content. All interactive elements built from `<div>` with no keyboard or screen reader access (overlap with accessibility, but rendering owns the structural cause). Images or dynamic content causing major visible layout shifts on primary user flows.
- **MEDIUM:** Missing skeleton screens on primary content loads. Heading hierarchy that skips levels or uses headings for styling. Device-specific breakpoints with no content-driven logic. Content unreachable without JavaScript but JS failure is unlikely.
- **LOW:** Minor heading hierarchy inconsistencies. Font loading causing brief text reflow with reasonable fallback. Breakpoint-based responsive design that works but isn't content-driven. Missing `aspect-ratio` on non-critical media.

## Output

After reviewing, produce both files per the shared output format in the orchestrator's output directory.
