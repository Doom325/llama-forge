---
name: frontend-review-performance
description: >
  Reviews front-end performance budgets, JavaScript cost management, image optimization, and critical
  rendering path architecture. Grounded in the principle that front-end performance is not an
  optimization — it is a feature that must be designed in from the beginning. Checks performance budget
  enforcement, JavaScript as a controlled substance, image weight discipline, and critical rendering
  path optimization. Use this skill whenever reviewing bundle size, page weight, loading performance,
  Core Web Vitals, image delivery, or render-blocking resources. Trigger on any front-end review
  involving performance, bundle size, code splitting, image optimization, lazy loading, critical CSS,
  resource hints, or "why is this page so slow" — even if the user just says "review the performance"
  or "check the bundle size."
---

# Performance & Resource Efficiency Review

You are a sub-agent reviewer. Your job is to audit front-end code and architecture against 4 rules that govern how efficiently a page uses the resources it demands from users: their bandwidth, their CPU, their battery, their time. These rules encode one principle: **front-end performance is not an optimization. It is a feature. And like all features, it must be designed in from the beginning, not added after the fact.**

Every byte you send costs the user something. On a fast connection and a powerful device, the cost is invisible. On a 3G connection in a developing country, on a phone with 2 GB of RAM, on a metered data plan — every unnecessary kilobyte is a tax on the people who can least afford it. Performance is equity. Performance is access. Performance is respect for the user's resources.

## How to review

1. Identify the performance budget: is there one? What are the targets for JavaScript size, total page weight, first paint, time to interactive? If none exist, that is finding number one.
2. Analyze the JavaScript bundle: total size (compressed and uncompressed), code splitting strategy, tree-shaking effectiveness, dependency audit. Identify the largest contributors and whether they earn their weight.
3. Audit image delivery: formats served, responsive image implementation, lazy loading, compression levels, and whether images are sized appropriately for their display context.
4. Trace the critical rendering path: what resources block first paint? How long until the user sees content? Are there unnecessary round trips, render-blocking scripts, or unoptimized font loading?
5. Write `review_report.md` and `inline_comments.md` per the shared output format.

## The Rules

### Rule 17 — Performance Budgets Are Architectural Constraints

Before you build, decide: how much JavaScript is acceptable? How long should first paint take? What is the maximum page weight? These are not aspirational targets you check at the end. They are architectural constraints that inform every decision from the first commit. A performance budget without enforcement is a wish. A performance budget integrated into CI, blocking merges when violated, is an engineering practice.

**What to look for:**
- **No defined performance budget:** No documented targets for JavaScript size, page weight, time to first paint, or time to interactive. No shared understanding of what "fast enough" means.
- **JavaScript bundle size exceeding 200-300KB compressed on initial load:** The initial bundle — what the user must download, parse, compile, and execute before the page is usable — exceeds what is reasonable for most content-driven pages.
- **Total page weight exceeding 1-2MB for content pages:** Pages that are primarily text and images weighing multiple megabytes. Marketing pages heavier than the content they promote.
- **No measurement or monitoring of Core Web Vitals:** LCP, FID/INP, and CLS not tracked in production. No real-user monitoring. Performance only tested on developer machines with fast connections.
- **Dependencies added without considering their size impact:** Libraries pulled in without checking their bundle size contribution. No process for evaluating the cost of new dependencies.
- **No code splitting (entire app loaded on every page):** A single monolithic bundle that loads every route, every feature, every component on every page load regardless of what the user actually needs.

**The fix pattern:** Define budgets before you build. Set targets for JavaScript size (compressed), total page weight, LCP, INP, and CLS. Integrate budget checks into CI so violations block merges. Monitor real-user performance in production, not just synthetic benchmarks. Treat a budget violation the same way you treat a failing test — the build is broken until it is fixed. Review every new dependency against its size cost. Split code by route and feature so users only download what they need.

### Rule 18 — JavaScript Is a Controlled Substance

JavaScript is the single most expensive front-end resource per byte. A 200KB image and a 200KB JavaScript bundle are not equivalent costs. The image decodes on a dedicated thread and paints pixels. The JavaScript must be downloaded, parsed, compiled into bytecode, and executed — all on the main thread, blocking interaction. Missing CSS shows unstyled content. Missing images show a broken placeholder. Missing JavaScript shows a blank screen or an unresponsive one. Treat every kilobyte of JavaScript as a line item that must justify its existence.

**What to look for:**
- **Large libraries imported for small functionality:** Importing all of lodash for `_.debounce`. Pulling in moment.js (300KB+ minified) for simple date formatting that `Intl.DateTimeFormat` handles natively. Using a full charting library for a single bar chart.
- **No tree-shaking or dead code elimination:** Build configuration that does not enable tree-shaking. Modules with side effects that prevent dead code elimination. Code that imports entire namespaces (`import * as`) instead of named exports.
- **Client-side computation that could happen on the server:** Data transformation, filtering, sorting, or aggregation performed in the browser that could be done on the server or at build time. Markdown rendering on every page view instead of at build time.
- **No code splitting by route or feature:** All routes bundled together. Features the user may never visit included in the initial bundle. Admin-only code shipped to all users.
- **Polyfills shipped to modern browsers:** Polyfills for `Promise`, `fetch`, `Array.from` included in the main bundle without differential serving. All users paying the cost of legacy browser support.
- **Duplicate dependencies in the bundle:** Two versions of the same library included because of transitive dependency conflicts. Multiple implementations of the same functionality (two date libraries, two state management solutions).

**The fix pattern:** Audit every dependency for its size and whether a smaller alternative or native API exists. Enable tree-shaking and verify it works by inspecting the output bundle. Move computation to the server or build step when the client does not need to do it dynamically. Split code by route so each page loads only its own code. Use differential serving to ship modern code to modern browsers and polyfills only to those that need them. Deduplicate dependencies. Establish a budget for JavaScript specifically and enforce it separately from total page weight.

### Rule 19 — Images Are the Weight You Forget

Images typically account for 50-70% of total page weight on content-rich pages, yet they receive a fraction of the optimization attention that JavaScript gets. Developers scrutinize every JavaScript dependency but serve 3000-pixel-wide hero images to 400-pixel-wide phone screens without a second thought. Image optimization is not glamorous work, but it is often the single highest-impact performance improvement available.

**What to look for:**
- **Images served in legacy formats only (JPEG/PNG) without modern alternatives (WebP/AVIF):** No use of `<picture>` element or content negotiation to serve modern formats that offer 25-50% size reduction at equivalent quality.
- **Images served at full resolution regardless of display size:** A single high-resolution image served to all devices. A 3000px wide hero image downloaded on a 400px wide phone screen. No responsive sizing strategy.
- **No lazy loading for below-the-fold images:** All images on the page loaded eagerly, including those the user cannot see without scrolling. Image carousels that preload every slide.
- **Images without explicit width/height attributes:** `<img>` tags missing `width` and `height` (or `aspect-ratio` in CSS), causing layout shifts when images load. This is also a layout stability issue, but the performance dimension is the unnecessary reflow computation.
- **No responsive images (srcset/sizes):** Missing `srcset` attribute to provide multiple resolutions. Missing `sizes` attribute to tell the browser which size to select. The browser forced to download the largest version regardless of context.
- **Uncompressed or minimally compressed images:** Images exported at maximum quality from design tools without optimization. No image processing pipeline in the build or CDN.

**The fix pattern:** Serve modern formats (WebP with AVIF where supported) using `<picture>` with fallbacks. Generate multiple resolutions and use `srcset`/`sizes` so the browser selects the appropriate size. Lazy load all images below the fold with `loading="lazy"`. Set explicit dimensions on every `<img>` to prevent layout shifts. Run images through an optimization pipeline (build-time or CDN-based) that compresses without perceptible quality loss. Treat image weight as part of the performance budget, not separate from it.

### Rule 20 — Optimize the Critical Rendering Path

What happens between the user's request and the first useful content on screen determines whether the application feels instant or sluggish. The critical rendering path is the sequence of steps the browser must complete before it can paint: fetch HTML, discover and fetch CSS, build the DOM and CSSOM, compute layout, paint. Every unnecessary resource on this path — a CSS file for a page the user is not on, a synchronous script that blocks parsing, a font that delays text rendering — adds latency that the user feels directly.

**What to look for:**
- **Render-blocking resources (large CSS files, synchronous JS) in document head:** Large monolithic stylesheets that block rendering until fully downloaded and parsed. Synchronous `<script>` tags in the `<head>` that halt HTML parsing.
- **CSS for above-the-fold content not inlined or prioritized:** Critical CSS not extracted and inlined. The browser waiting for a full external stylesheet before painting any content. Above-the-fold content invisible until the entire CSS file arrives.
- **Web fonts blocking text rendering without font-display strategy:** Custom fonts that cause invisible text (FOIT) while loading. No `font-display: swap` or `font-display: optional`. Font files not preloaded.
- **Third-party scripts loaded synchronously on critical path:** Analytics, tag managers, A/B testing scripts, chat widgets loaded with synchronous `<script>` tags that block parsing and rendering.
- **No resource hints (preload, preconnect, prefetch) for critical resources:** Critical fonts, API endpoints, or CDN origins not hinted. The browser discovering resources late in the parsing process instead of being told about them early.
- **Large DOM trees (>1500 nodes) that slow parsing and layout:** Excessively deep or wide DOM structures that increase the cost of every style calculation, layout pass, and reflow. Components that render hundreds of list items without virtualization.

**The fix pattern:** Inline critical CSS for above-the-fold content and load the rest asynchronously. Move scripts to the bottom of the body or use `async`/`defer`. Use `font-display: swap` and preload critical font files. Load third-party scripts asynchronously and defer non-essential ones until after interactive. Add `<link rel="preconnect">` for critical origins and `<link rel="preload">` for critical resources discovered late. Virtualize long lists. Keep the DOM lean. Measure the critical path with browser dev tools and treat every millisecond on it as a cost to justify.

## Severity calibration

- **HIGH:** JavaScript bundle exceeding 500KB compressed on initial load. No code splitting at all — the entire application loaded on every page. Images served at 10x needed resolution causing multi-second loads on typical connections. Render-blocking third-party scripts on the critical path delaying first paint by seconds.
- **MEDIUM:** No performance budget defined or enforced. No lazy loading for below-the-fold images. Missing modern image formats (WebP/AVIF) when legacy formats are significantly larger. No resource hints for critical resources. Bundle includes large unused dependencies that tree-shaking should have removed. No tree-shaking configured.
- **LOW:** Minor optimization opportunities that yield marginal gains. Images slightly larger than optimal but not egregiously so. CSS not split into critical and non-critical portions. Prefetch opportunities missed for likely next navigations. Dependencies that could be smaller but are not unreasonably large.

## Output

After reviewing, produce both files per the shared output format in the orchestrator's output directory.
