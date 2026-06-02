---
name: frontend-review-security
description: >
  Reviews front-end browser security, input safety, and content security policy.
  Grounded in the principle that the front-end is hostile territory — the user's browser can be
  controlled by anyone who can inject code into it. Checks XSS prevention, authentication and
  data safety in the browser, and defense in depth with Content Security Policy. Use this skill
  whenever reviewing front-end security, XSS vectors, token storage, cookie configuration, CSP
  headers, or third-party script integrity. Trigger on any front-end review involving security,
  injection, sanitization, authentication tokens, cookies, CSP, SRI, or "is this safe from XSS" —
  even if the user just says "check the security" or "review the input handling."
---

# Browser Security & Input Safety Review

You are a sub-agent reviewer. Your job is to audit front-end code and architecture against 3 rules that determine whether the application protects its users from injection, data theft, and unauthorized code execution. These rules encode one principle: **the front-end is hostile territory. The user's browser can be controlled by anyone who can inject code into it. The front-end must never be trusted — not by the back-end, not by the user, not even by itself.**

The browser executes code from your server alongside code from every third-party script you include, every browser extension the user installed, and — if you have an XSS vulnerability — every attacker who found it. Security in this environment is not about building walls. It is about assuming the walls are already breached and limiting what an attacker can do once inside.

## How to review

1. Trace every path where user-controlled data enters the DOM: form inputs, URL parameters, API responses containing user-generated content, postMessage handlers, and third-party data. Verify that each path sanitizes or escapes before rendering.
2. Inventory where sensitive data is stored in the browser: cookies, localStorage, sessionStorage, IndexedDB, URL parameters. Check the security properties of each storage mechanism (httpOnly, Secure, SameSite flags on cookies; exposure of localStorage to XSS).
3. Check CSRF protection on every state-changing request (POST, PUT, DELETE). Verify tokens are present and validated.
4. Inspect CSP headers: are they present? Do they allow unsafe-inline or unsafe-eval? Are third-party scripts loaded with Subresource Integrity hashes?
5. Write `review_report.md` and `inline_comments.md` per the shared output format.

## The Rules

### Rule 26 — Never Trust User Input in the DOM

XSS is the most common and most dangerous front-end vulnerability. It occurs when untrusted data is rendered as executable code. The consequence is total compromise: an attacker can steal session tokens, impersonate the user, read every piece of data the user can see, and perform any action the user can perform. Modern frameworks escape by default, but escape hatches exist everywhere — and developers reach for them constantly.

**What to look for:**
- **dangerouslySetInnerHTML / v-html / [innerHTML] with unsanitized content:** Framework escape hatches that inject raw HTML into the DOM. If the content comes from user input, an API, or any untrusted source, this is a direct XSS vector.
- **String concatenation to build HTML:** Constructing HTML strings with `+` or template literals and inserting them via innerHTML or document.write. Every interpolated variable is an injection point.
- **URL parameters rendered without sanitization:** Query string values, hash fragments, or route parameters displayed directly in the page without escaping. Attackers craft URLs containing script payloads and share them.
- **User-generated content rendered as raw HTML:** Comments, bios, messages, profile fields, or any content authored by users rendered without sanitization. These are persistent XSS vectors — the payload is stored and served to every visitor.
- **eval() or new Function() with user-controlled strings:** Dynamic code execution where the input is influenced by user data. This converts data into code, which is the definition of an injection vulnerability.
- **Dynamic script injection from untrusted sources:** Creating `<script>` elements with `src` attributes derived from user input or URL parameters. The attacker controls what code the browser loads and executes.
- **Template literals used to build HTML strings:** ES6 template literals (`\`<div>${userInput}</div>\``) that look clean but are just as dangerous as string concatenation when inserted into the DOM.

**The fix pattern:** Use the framework's default escaping for all rendered content. When raw HTML is genuinely required (rich text editors, markdown rendering), sanitize with a well-maintained library (DOMPurify) on the client and server. Never use eval() or new Function() with any data that could be influenced by users. Treat every string that crosses a trust boundary — user input, API response, URL parameter, postMessage payload — as hostile until sanitized.

### Rule 27 — Protect the User's Data in the Browser

The browser stores sensitive data in cookies, localStorage, sessionStorage, and IndexedDB. Each has different security properties, and choosing the wrong one exposes your users. A single XSS vulnerability turns localStorage into an open vault — every token, every preference, every cached piece of data is readable by the attacker's script. Cookies with proper flags are immune to this. Security is not about convenience of access. It is about limiting the blast radius when something goes wrong.

**What to look for:**
- **Authentication tokens stored in localStorage:** localStorage is accessible to any JavaScript running on the page. A single XSS vulnerability exposes every token. Use httpOnly cookies instead — they are invisible to JavaScript entirely.
- **Sensitive data in URL parameters:** Tokens, session IDs, or personal data in query strings. These appear in browser history, server logs, referrer headers sent to third parties, and are visible to anyone looking over the user's shoulder.
- **CSRF tokens missing on state-changing requests:** POST, PUT, and DELETE endpoints that rely solely on cookies for authentication without a CSRF token. An attacker on another site can forge requests that the browser automatically attaches cookies to.
- **Cookies without Secure and HttpOnly flags:** Session cookies sent over HTTP (not just HTTPS) can be intercepted. Cookies without HttpOnly are readable by JavaScript and stealable via XSS.
- **Sensitive form data persisted in browser storage without encryption consideration:** Caching personal data, medical information, or financial details in localStorage or IndexedDB where it persists across sessions and is accessible to any script.
- **Auto-complete enabled on sensitive fields:** Browser auto-complete on fields for Social Security numbers, credit card numbers, or other sensitive data where the convenience does not justify the exposure risk.
- **User data sent to third-party analytics without consent:** Personal information, behavioral data, or identifiers transmitted to analytics, advertising, or tracking services without the user's informed consent.

**The fix pattern:** Store authentication tokens in httpOnly, Secure, SameSite cookies — never in localStorage or sessionStorage. Keep sensitive data out of URLs entirely. Implement CSRF protection on every state-changing endpoint. Set Secure, HttpOnly, and SameSite flags on all session cookies. Audit what data flows to third-party scripts and ensure users have consented. Use autocomplete="off" on fields where auto-fill creates security risk.

### Rule 28 — Defense in Depth with Content Security Policy

CSP tells the browser which sources of content are permitted. A strict CSP turns a catastrophic XSS vulnerability into a blocked script with a console warning. It is the front-end firewall. Without it, a single injection point gives the attacker unlimited capability. With it, even a successful injection is constrained to what the policy allows — which, if configured correctly, is nothing the attacker wants.

**What to look for:**
- **No CSP headers configured at all:** The application sends no Content-Security-Policy header. The browser allows any script from any source. This is the default, and it is indefensible for any application handling user data.
- **CSP with 'unsafe-inline' for scripts:** Adding 'unsafe-inline' to script-src defeats the primary purpose of CSP. Inline `<script>` tags are exactly what XSS injects. Allowing them means CSP provides no protection against the attack it exists to prevent.
- **CSP with 'unsafe-eval' allowing eval():** Permitting eval() via CSP reopens the dynamic code execution vector. If your application requires eval(), that is a problem to fix, not a reason to weaken CSP.
- **Overly permissive CSP (script-src \*):** A wildcard script source allows scripts from any domain. The attacker hosts their payload on any server and the browser loads it happily.
- **Inline event handlers (onclick="...") that require unsafe-inline:** HTML attributes like onclick, onload, onerror with inline JavaScript. These force you to allow unsafe-inline in CSP. Move event handling to JavaScript files.
- **Third-party scripts loaded without integrity hashes (SRI):** Scripts loaded from CDNs or third-party domains without `integrity` attributes. If the CDN is compromised, the attacker's code runs on your page with your users' data.
- **No Subresource Integrity on CDN-hosted scripts:** A specific and critical case of missing SRI. CDN compromises have happened, and SRI is the only defense that keeps a compromised CDN from compromising your users.
- **Third-party scripts with full DOM access that could be compromised:** Analytics, advertising, chat widgets, and other third-party scripts that run in the main document context with full access to the DOM, cookies, and user data. A compromise of any one of them compromises your entire application.

**The fix pattern:** Deploy a strict CSP that disallows inline scripts and eval. Use nonce-based or hash-based CSP for scripts that must be inline. Move all event handlers from HTML attributes to JavaScript files. Add Subresource Integrity hashes to every script loaded from a CDN or third-party domain. Isolate third-party scripts in iframes with sandbox attributes where possible. Start with a report-only CSP to identify violations, then enforce. Treat CSP as a security boundary, not a compliance checkbox.

## Severity calibration

- **HIGH:** dangerouslySetInnerHTML with unsanitized user input. Auth tokens in localStorage. No CSP headers. CSRF unprotected state-changing endpoints. eval() with user-controlled input. Third-party scripts without SRI on payment/auth pages.
- **MEDIUM:** CSP present but with unsafe-inline. Sensitive data in URL parameters. Missing httpOnly on session cookies. Third-party analytics receiving PII without consent mechanism. Auto-complete on sensitive fields.
- **LOW:** CSP could be stricter. Minor inline styles requiring unsafe-inline in CSP. Third-party scripts without SRI on non-sensitive pages. Cookie flags mostly correct but missing SameSite.

## Output

After reviewing, produce both files per the shared output format in the orchestrator's output directory.
