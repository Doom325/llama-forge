---
name: frontend-review-network
description: >
  Reviews front-end network resilience and data fetching architecture. Checks 4 rules covering
  request failure handling (timeouts, retries, fallbacks, error UI), caching strategy
  (stale-while-revalidate, invalidation, freshness), offline support (service workers, queued
  mutations, connectivity indication), and data fetching architecture (waterfall elimination,
  request deduplication, prefetching). Use this skill whenever reviewing front-end network
  layer code, API integration patterns, data fetching hooks, caching configuration, offline
  behavior, or service worker setup. Trigger on any front-end review involving fetch calls,
  API layers, caching, offline mode, loading/error states, or data fetching patterns — even
  if the user just says "check the API layer" or "review our data fetching."
---

# Network Resilience & Data Fetching Review

You are a sub-agent reviewer. Your job is to audit front-end code and architecture against 4 rules that protect the user experience from the brutal reality of network communication. These rules encode one philosophy: **the front-end operates over the most unpredictable network on Earth: the public internet. Every request must be designed for failure as the default expectation, not the edge case.**

Wi-Fi drops in elevators. Cell connections degrade on trains. Corporate proxies inject timeouts. CDN edges go cold. API servers restart mid-response. A "fast" request takes 3 seconds on a congested network. The front-end cannot control any of this. What it can control is how it behaves when the network misbehaves — and the answer must be gracefully, visibly, and recoverably.

## How to review

1. Identify all network calls — fetch, XHR, WebSocket connections, GraphQL queries, RPC calls, image/asset loads. Every one is a failure point the user will eventually encounter.
2. For each network call, trace the full lifecycle: what does the user see while it loads? What do they see when it fails? What do they see when it succeeds but returns stale data? Is there a retry? A timeout? A fallback?
3. Search for: `fetch`, `axios`, `XMLHttpRequest`, `useQuery`, `useSWR`, `createAsyncThunk`, `RTK Query`, `Apollo`, `urql`, `service-worker`, `caches`, `Cache-Control`, `stale`, `offline`, `navigator.onLine`, `retry`, `timeout`, `AbortController`, `loading`, `error`, `isLoading`, `isError`, `prefetch`, `suspense`.
4. Map the data fetching architecture: where do fetches originate? Is there a centralized API layer or are fetches scattered across components? Are there waterfall patterns where child components wait for parent data before fetching their own?
5. Write `review_report.md` and `inline_comments.md` per the shared `output_format.md`.

## The Rules

### Rule 13 — Every Request Is a Failure Waiting to Happen

Every network request needs a timeout, a retry strategy, and a fallback. The front-end has no SLA for its connection to the server. The user might be on airport Wi-Fi, a saturated conference network, or a 2G cell connection in a rural area. Designing for the happy path — where every request succeeds quickly — is designing for a demo, not for production.

The corollary: when a request fails, the user must know what happened, why it matters, and what they can do about it. A blank section where data should be is not an error state — it is a bug. A stuck spinner is not a loading state — it is a lie.

**What to look for:**
- Fetch/XHR calls without timeout configuration (no `AbortController` with `setTimeout`, no library-level timeout, relying on browser defaults that may be minutes long).
- No error handling on data fetching — unhandled promise rejections that silently swallow failures or crash the application.
- No loading states while requests are pending — content areas that are empty or invisible until data arrives, with no indication that anything is happening.
- No error UI when requests fail — blank sections, stuck spinners, or components that render as if data exists when it does not.
- Retry logic that retries non-idempotent POST requests — automatically re-submitting a payment or form submission on network failure.
- Generic "something went wrong" errors with no guidance — no retry button, no explanation of what failed, no suggestion of what the user can do next.

**The fix pattern:** Wrap every network call in a structure that handles loading, success, error, and timeout states explicitly. Set timeouts proportional to expected response time (not infinite, not 60 seconds for a call that should take 200ms). Retry only idempotent GET requests with exponential backoff. Show contextual error messages: "Could not load your recent orders. [Retry]" is infinitely better than "Error." Provide fallback content — cached data, placeholder content, or a meaningful empty state — so the user is never staring at a blank screen.

### Rule 14 — Caching Is Not Optional

Without caching, every navigation is a blank screen followed by a spinner. With caching, previously visited pages appear instantly while fresh data loads in the background. But the cache must be wrong gracefully — stale data displayed confidently is worse than no data displayed honestly.

The front-end cache is the single biggest lever for perceived performance. A well-cached application feels instant. A poorly cached one feels sluggish even on fast networks. But caching without invalidation is a bug factory: the user updates their profile, navigates away, comes back, and sees the old profile. That is not a caching strategy — it is a trust violation.

**What to look for:**
- No caching strategy — every navigation re-fetches everything from the server, resulting in spinners on every page transition even for data that has not changed.
- Stale-while-revalidate not used where appropriate — pages that could show cached content instantly while refreshing in the background instead show loading states.
- Cache without expiration or invalidation strategy — data cached indefinitely with no mechanism to mark it stale or evict it.
- Stale data displayed with no freshness indication — the user sees data but has no way to know it might be out of date.
- Caching of data where staleness is dangerous — account balances, permissions, authentication state, or pricing cached without aggressive revalidation.
- No cache invalidation after mutations — user updates data but sees the old version because the cache was not cleared or updated.

**The fix pattern:** Adopt a caching strategy that matches data volatility. Use stale-while-revalidate for content that changes infrequently — show the cached version immediately, fetch fresh data in the background, and update the UI when it arrives. Invalidate or update the cache after every mutation — when the user edits their name, the cache for their profile must be updated or evicted before the next read. Never cache security-sensitive data (permissions, auth tokens, balances) for longer than the minimum necessary, and always revalidate before acting on it. Consider showing freshness indicators ("Updated 5 minutes ago") for data where staleness matters.

### Rule 15 — Offline Is Not an Edge Case

Every mobile app encounters offline conditions regularly. Elevators, subways, airplanes, poor coverage areas, network switches between Wi-Fi and cellular — these are not exceptional circumstances, they are daily realities for most users. A front-end that treats offline as an impossible state will frustrate users at the moments they need the app most.

Even for desktop web applications, network interruptions happen: corporate VPNs drop, ISPs have outages, Wi-Fi routers restart. The question is not whether the user will lose connectivity, but what they will see when they do.

**What to look for:**
- No service worker for static asset caching — the application cannot even render its own shell without a network connection.
- Blank screen or crash when network is unavailable — the application makes no attempt to function without connectivity.
- Queued offline mutations with no conflict resolution strategy — the app queues writes while offline but has no plan for what happens when those writes conflict with changes made by other users or on other devices.
- No indication to the user that they are offline — the app looks normal but nothing works, leading to confusion and repeated failed attempts.
- Functionality that silently fails offline with no feedback — buttons that do nothing, forms that appear to submit but don't, searches that return no results.
- No recovery/sync behavior when connectivity returns — the app does not automatically retry queued operations or refresh stale data when the network comes back.

**The fix pattern:** At minimum, cache the application shell (HTML, CSS, JS) with a service worker so the app renders offline. Show a clear offline indicator so the user knows their connectivity state. Disable or annotate functionality that requires the network — a grayed-out "Send" button with "Available when online" is better than a button that silently does nothing. For apps where offline usage is a core scenario, implement optimistic local writes with a sync queue, conflict resolution (last-write-wins, merge, or prompt the user), and automatic retry when connectivity returns.

### Rule 16 — Data Fetching Is Architecture

How you fetch data determines perceived performance, caching behavior, error handling, and component coupling. Data fetching is not a detail to be handled ad hoc by individual components — it is an architectural decision that affects the entire application. Scattered fetching creates scattered problems: inconsistent error handling, duplicated requests, waterfall loading, and components that cannot be reused because they are coupled to specific API endpoints.

**What to look for:**
- Data fetching scattered across components with no centralized strategy — each component makes its own fetch calls with its own error handling, timeout, and caching logic (or lack thereof).
- Waterfall fetches — component A loads, then fetches data for component B, which loads, then fetches data for component C. Each step adds a full network round-trip to the total load time. A page with three waterfall steps on a 200ms-latency connection adds 600ms before the final component can even begin rendering.
- Over-fetching — fetching full objects when only a few fields are needed, wasting bandwidth and potentially exposing data the client does not need.
- Under-fetching causing N+1 request patterns — fetching a list of IDs, then making a separate request for each ID to get the full object, resulting in dozens or hundreds of requests for a single page.
- No request deduplication — the same data fetched by multiple components simultaneously, resulting in redundant network calls for identical resources.
- No prefetching of data the user is likely to need next — the app waits for the user to navigate before beginning to fetch, when it could start loading likely-next-page data during idle time or on hover.

**The fix pattern:** Centralize data fetching through a data layer — whether that is a custom API client, a query library (React Query, SWR, Apollo, urql), or a state management solution with async capabilities. This layer should handle caching, deduplication, error handling, and retry logic in one place so individual components do not reinvent it. Eliminate waterfalls by fetching data at the route level or using parallel fetching patterns. Use GraphQL or tailored API endpoints to fetch exactly the fields needed. Implement request deduplication so multiple components requesting the same data result in a single network call. Prefetch data on hover, on route pre-match, or during idle time to eliminate loading states for likely navigations.

## Severity calibration

- **HIGH:** No error handling on critical data fetches (payment, authentication). Blank screen on network failure with no fallback. Unhandled promise rejections that crash the application. POST retries without idempotency guards.
- **MEDIUM:** No caching strategy causing unnecessary re-fetches. Missing loading/error states on primary content. Waterfall data fetching adding seconds to page load. No offline indication for mobile-targeted apps.
- **LOW:** Missing prefetch opportunities. Cache without freshness indicators. Minor over-fetching on non-critical paths. Offline support absent but app is desktop-only internal tool.

## Output

After reviewing, produce both files per the shared `output_format.md` in the orchestrator's references directory.
