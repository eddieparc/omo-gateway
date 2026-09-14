# Lane: web-frontend

## Scope
- `web/src/App.tsx`: 1,514 LOC
- `web/src/api.ts`: 238 LOC
- `web/src/pages/BotsPage.tsx`: 395 LOC
- `web/src/components/ui/dialog.tsx`: 23 LOC
- `web/src/components/ui/button.tsx`: 52 LOC
- `web/src/components/ui/badge.tsx`: 37 LOC
- `web/src/components/ui/card.tsx`: 75 LOC
- `web/src/components/ui/input.tsx`: 24 LOC
- `web/src/components/ui/separator.tsx`: 20 LOC
- `web/src/components/ui/table.tsx`: 103 LOC
- `web/src/components/ui/tabs.tsx`: 90 LOC
- `web/src/components/ui/index.ts`: 8 LOC
- `web/src/main.tsx`: 10 LOC
- `web/src/index.css`: 64 LOC

## TypeScript Compiler Evidence
```
$ cd /Users/indo/code/project/omon-gateway/web && npx tsc -b --noEmit 2>&1 | head -40
(no output - clean build)
```

## Findings

### [P0] Chat WebSocket connection lacks token authentication or auth query parameter
- Location: web/src/App.tsx:555
- Evidence: `const ws = new WebSocket(socketUrl(`/api/sessions/${encodeURIComponent(currentSessionId)}/ws`))`
- Why it matters: `socketUrl(...)` builds a raw `ws://` or `wss://` URL without appending any session token, auth ticket, or bearer header. If the gateway server enforces authentication or session tokens (like `web/src/lib/api.ts`'s ticket-based auth), WebSocket handshake fails immediately with 401/403 or runs entirely unauthenticated, allowing any unauthorized local client to send prompts and execute tools on behalf of any session.
- Suggested fix: Pass the session token or use an authenticated WebSocket builder (e.g. `api.buildWsUrl` or appending `?token=...`).

### [P0] Stop turn creates an unauthenticated orphaned WebSocket connection with hardcoded 500ms closure
- Location: web/src/App.tsx:624
- Evidence: `const ws = new WebSocket(socketUrl(`/api/sessions/${encodeURIComponent(currentSessionId)}/ws`))`
- Why it matters: Every time the user clicks "Stop Turn", a new WebSocket is opened solely to send `{"type": "stop"}`. The socket has no error handling, no auth token, and attempts a blind `setTimeout(() => ws.close(), 500)`. If the network is slow or connection fails, the socket can remain open indefinitely or fail before delivering the cancel command, leaving runaway agent tasks executing on the server.
- Suggested fix: Re-use the existing active WebSocket connection reference to send the stop signal, or invoke a dedicated REST endpoint (e.g. `POST /api/sessions/{id}/stop`).

### [P0] Frontend API client omits session tokens and custom headers on all HTTP requests
- Location: web/src/api.ts:128
- Evidence: `async function request<T>(path: string, init?: RequestInit): Promise<T> {`
- Why it matters: The primary API client used by `App.tsx` and `BotsPage.tsx` (`web/src/api.ts`) constructs raw `fetch()` calls without attaching `X-Hermes-Session-Token` or any authorization header, and without `credentials: "include"`. In environments where the gateway requires token authentication or cookie-based gating, all dashboard API requests fail with 401 Unauthorized.
- Suggested fix: Read `window.__HERMES_SESSION_TOKEN__` and attach `X-Hermes-Session-Token` header, or unify with `web/src/lib/api.ts`'s `fetchJSON`.

### [P0] Live logs WebSocket lacks reconnect logic and crashes on unhandled socket errors
- Location: web/src/App.tsx:1434
- Evidence: `const ws = new WebSocket(socketUrl('/api/logs/ws'))`
- Why it matters: The telemetry WebSocket in `LiveLogsPage` connects once inside `useEffect([], ...)`. It sets no `ws.onerror` handler and has no auto-reconnection loop on `onclose`. If the gateway daemon restarts, drops the connection, or encounters network jitter, `connected` flips to false and streaming permanently halts without user feedback or recovery attempt.
- Suggested fix: Add an exponential-backoff reconnect handler on `onclose`/`onerror` or provide a manual "Reconnect" action.

### [P1] Missing cleanup and race condition on session message fetching in ChatPlaygroundPage
- Location: web/src/App.tsx:524
- Evidence: `useEffect(() => {`
- Why it matters: When switching sessions rapidly in the sidebar, `loadMessages(currentSessionId)` is called asynchronously for each clicked session. Because there is no cancellation token (`cancelled = true` or `AbortController`), an earlier slow network response can resolve after a faster newer one, overwriting `messages` with another session's history and leading to desynchronized chat contexts.
- Suggested fix: Use an active/cancelled flag inside `useEffect` or an `AbortController` to abort stale in-flight fetches when `currentSessionId` changes.

### [P1] Missing debounce or abort on search input creates request storms and race conditions
- Location: web/src/App.tsx:1155
- Evidence: `useEffect(() => {`
- Why it matters: In `SessionsExplorerPage`, `load()` is called directly inside `useEffect` with `search` as a dependency. Every keystroke fires an unthrottled `GET /api/sessions?search=...` request. If the backend responses arrive out of order, the table displays stale search results instead of the latest search query.
- Suggested fix: Debounce `search` state changes by 250ms and cancel previous pending requests via `AbortController`.

### [P1] ReactMarkdown code block component unescapes HTML props via loose any casting
- Location: web/src/App.tsx:817
- Evidence: `code: ({ node, inline, children, ...props }: any) => {`
- Why it matters: In `ChatPlaygroundPage`, the `code` component overrides `children` and passes `...props` directly to the `<code>` tag. Because props are cast to `any`, untyped attributes from custom markdown plugins or backend formatting payloads bypass React typechecking and can lead to runtime prop mismatch or DOM attribute pollution.
- Suggested fix: Explicitly type the code component props using `ExtraProps` from `react-markdown` without using `any`.

### [P1] Unconditional scrollIntoView on every message chunk disrupts user reading during streaming
- Location: web/src/App.tsx:531
- Evidence: `messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })`
- Why it matters: Whenever `streamingContent` updates (which happens on every streaming token received over WebSocket), `scrollIntoView` forces the viewport to scroll to the bottom. If a user tries to scroll up to read earlier responses or tool execution results while the model is answering, their scroll position is constantly ripped back to the bottom.
- Suggested fix: Check whether the user is scrolled near the bottom (e.g., `scrollTop + clientHeight >= scrollHeight - 50`) before invoking `scrollIntoView`.

### [P2] Pending approvals badge in header lacks accessible role and keyboard activation
- Location: web/src/App.tsx:247
- Evidence: `onClick={() => selectPage('settings')}`
- Why it matters: The Pending Approvals warning badge acts as an interactive navigation trigger (`onClick`), but is rendered as a non-interactive `<span>` (the underlying `Badge` component) without `role="button"`, `tabIndex={0}`, or `onKeyDown` handlers. Screen readers and keyboard-only users cannot discover or trigger navigation to the security review page.
- Suggested fix: Wrap the badge in a `<button>` or add `role="button"`, `tabIndex={0}`, and `onKeyDown={(e) => e.key === 'Enter' && selectPage('settings')}`.

### [P2] Modal dialog backdrop lacks escape key listener and focus trapping
- Location: web/src/components/ui/dialog.tsx:10
- Evidence: `export function Dialog({ open, onOpenChange, children }: DialogProps) {`
- Why it matters: The custom `Dialog` implementation only closes when clicking the outer backdrop element (`onClick={() => onOpenChange(false)}`). It does not listen for the `Escape` key (`keydown`), does not lock body scrolling, and does not trap keyboard focus inside the dialog, violating WAI-ARIA modal dialog accessibility patterns.
- Suggested fix: Add a `keydown` event listener for `Escape` and use focus-trap or standard Radix dialog primitives.

### [P2] Telemetry poll error state retains stale status values
- Location: web/src/App.tsx:296
- Evidence: `if (!status) {`
- Why it matters: When the gateway goes offline, `refreshStatus` catches the error and sets `statusError`, but leaves `status` holding its previous snapshot. The overview screen continues rendering outdated active bot counts and model data instead of showing a degraded or offline warning banner.
- Suggested fix: Render an offline or reconnection banner when `statusError` is non-null even if `status` exists.

### [P2] Unbounded message list rendering without virtualization
- Location: web/src/App.tsx:735
- Evidence: `{messages.map((m) => {`
- Why it matters: In sessions with hundreds or thousands of messages, rendering the full array into the DOM simultaneously degrades render performance, increases memory usage, and causes scrolling stutter on low-power devices.
- Suggested fix: Implement windowing or list virtualization (e.g. `@tanstack/react-virtual`) or paginate message history retrieval.

### [P2] Dual codebase divergence between web/src and web/src/lib/api
- Location: web/src/pages/ChatPage.tsx:29
- Evidence: `import { useCallback, useEffect, useMemo, useRef, useState } from "react";`
- Why it matters: The repository contains two parallel dashboard implementations: the Shadcn/Tailwind UI in `web/src/App.tsx` + `web/src/api.ts` (configured in `tsconfig.app.json`), and the legacy Hermes dashboard pages (`web/src/pages/ChatPage.tsx`, `web/src/lib/api.ts`, etc.) imported from external packages. Changes made to one API client or page do not affect the active entry point, causing contract drift and confusion for maintainers.
- Suggested fix: Remove dead legacy pages or fully migrate all features to the new single architecture.

### [P2] Window alert and confirm dialogs block browser UI thread
- Location: web/src/pages/BotsPage.tsx:42
- Evidence: `alert(`Failed to load bots: ${err.message}`)`
- Why it matters: Using synchronous `window.alert()` and `window.confirm()` halts JavaScript execution and freezes the entire browser tab UI thread. In modern responsive SPAs, this provides poor UX compared to non-blocking toast notifications and modal confirmation dialogs.
- Suggested fix: Replace `alert()` and `confirm()` with a toast notification hook or the existing `ConfirmDialog` component.

### [P2] Form inputs lack accessibility labels
- Location: web/src/App.tsx:1104
- Evidence: `<label className="text-xs font-medium text-muted-foreground">Job ID</label>`
- Why it matters: In the Cron creation modal, `<label>` elements are not associated with their respective `<Input>` components via `htmlFor` / `id` attributes or `aria-label`, preventing screen readers from announcing field descriptions when inputs receive focus.
- Suggested fix: Add `htmlFor="job-id"` on `<label>` and matching `id="job-id"` on `<Input>`.

### [P2] Missing empty state UI on capabilities lists
- Location: web/src/App.tsx:1233
- Evidence: `api.tools().then((t) => setTools(t.items || []))`
- Why it matters: If the gateway returns empty lists for tools or skills (`items: []`), the capabilities cards render completely empty white containers with no explanatory empty-state text or instructions on how to register capabilities.
- Suggested fix: Add fallback empty state notices when `tools.length === 0` or `skills.length === 0`.

## Strengths
- The UI features a clean, responsive Shadcn-inspired layout with dark mode aesthetics and consistent typography.
- Markdown rendering in assistant responses safely filters and renders code blocks, image attachments, and tool call invocations.
- The state management uses native React hooks with clean separation of concerns across pages and components.
- Live telemetry streaming provides real-time system monitoring with log level color coding and automatic scrolling.

## Notes
- `tsconfig.app.json` only includes `src/main.tsx`, `src/App.tsx`, and `src/api.ts`. The files under `src/pages/*` (except `BotsPage.tsx`) and `src/lib/*` are residual artifacts from a legacy Hermes dashboard fork and are not compiled in the Vite production build.
