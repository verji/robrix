# Research: Matrix Widget Support in Robrix

## Context

Matrix widgets are web-based applications embedded inside a Matrix client. They communicate
with the host client via the Matrix Widget API (postMessage-based protocol). Widgets are not
yet part of the official Matrix spec but have a de facto specification and are widely used
(polls, video calls via Element Call, collaborative editors, etc.).

Robrix is a native Rust application built with Makepad. It does not use a browser engine,
which makes hosting web widgets a non-trivial challenge. This document tracks our research
into feasibility, complexity, and potential approaches.

---

## Research Areas

### 1. Matrix Widget API Specification
**Status:** Not started
**Goal:** Understand what a widget host must implement.

Questions to answer:
- What is the current state of the widget spec? Which MSCs define it?
  (MSC1236, MSC2762, MSC2871, MSC2764, MSC3819, others?)
- What does the host-to-widget communication protocol look like?
  (postMessage over iframe boundary, message schemas, request/response)
- What capabilities must a host provide?
  (permissions model, sending/receiving Matrix events, openID tokens, navigation, etc.)
- What is the minimum viable subset a host needs to support to be useful?
- Are there versioning or capability-negotiation mechanisms?

Findings:
> _(to be filled in)_

---

### 2. Existing Implementations
**Status:** Not started
**Goal:** Learn from how other clients host widgets.

Questions to answer:
- How does Element Web host widgets? (iframe + postMessage — reference implementation)
- How do Element Android / Element iOS (Element X) handle it?
  (Do they use a WebView? How is the JS bridge done?)
- Is there a standalone `matrix-widget-api` JS SDK or library?
  What does it provide — host-side driver, widget-side driver, or both?
- Are there any non-web native implementations of a widget host?
- What does the matrix-rust-sdk provide, if anything, for widget support?

Findings:
> _(to be filled in)_

---

### 3. Web Content Embedding in Rust
**Status:** In progress
**Goal:** Identify how to render web content inside a Makepad/Rust application.

#### 3a. Rust Webview Crate Landscape

| Crate | Version | Engine(s) | Platforms | JS<->Rust | Size Impact | Status |
|-------|---------|-----------|-----------|-----------|-------------|--------|
| **wry** | 0.54.2 | WebView2, WKWebView, WebKitGTK, Android WebView | Win/Mac/Linux/iOS/Android | `window.ipc.postMessage()` + `evaluate_script()` | Minimal (OS webview) | Active (Tauri team) |
| **tao** | 0.34.5 | N/A (windowing) | Win/Mac/Linux/iOS/Android | N/A | Minimal | Active (Tauri team) |
| **webkit2gtk** | 2.0.2 | WebKitGTK | Linux only | `run_javascript()` + signals | Minimal (system lib) | Active |
| **webview2-com** | 0.38.2 | WebView2 (Chromium Edge) | Windows only | `PostWebMessage*()` + `ExecuteScript()` | Minimal (system runtime) | Active |
| **cef** (cef-rs) | 145.2.0 | Full Chromium (CEF 145) | Win/Mac/Linux (x64+ARM64) | CEF V8 + IPC | **Very large** (~100+ MB) | Active (Tauri team) |
| **Servo** | 0.0.3 | Servo (Rust-native) | Win/Mac/Linux | WebViewDelegate (WIP) | **Large** (full engine) | Active (Igalia) |
| **web-view** | 0.7.3 | MSHTML/IE, EdgeHTML, WebKit | Win/Mac/Linux | `window.external.invoke()` + `eval()` | Minimal | **Unmaintained** |

**Key takeaway:** `wry` is the clear leader for windowed webviews — but for offscreen/texture
rendering, we need to look at engines that can render without a visible window (see 3g below).

#### 3b. Makepad + Webview Integration — No Precedent Exists

- **Makepad has no built-in webview** or browser component.
- **Nobody has publicly embedded a webview in Makepad** before.
- **Makepad uses its own custom windowing** — it does NOT use winit. The developers explicitly
  rejected winit, implementing their own platform layers (`win32_window.rs`, `macos_window.rs`,
  `xlib_window.rs`).
- **Native handles exist internally but are not exposed publicly:**
  - Windows: `Win32Window.hwnd: HWND` (pub within crate)
  - macOS: `MacosWindow.view: ObjcId` / `window: ObjcId` (pub(crate))
  - Linux: X11 Window handle (internal)
- **Makepad does NOT implement `HasWindowHandle`** (the `raw-window-handle` trait). The
  `raw-window-handle` crate is not even a dependency.
- **No child window infrastructure** exists — Makepad manages top-level windows only.

#### 3c. Two Possible Integration Approaches

**Approach A: Child Window Overlay (using wry)**

Place a native webview on top of a Makepad widget area using OS-level window compositing.

| Pros | Cons |
|------|------|
| Uses lightweight system webview | Makepad doesn't expose native handles (needs fork or upstream PR) |
| Full web standards support | No layout coordination — must manually sync position/size |
| Simpler to implement initially | Z-ordering issues (webview always on top of GPU content) |
| | Linux incompatible (webkit2gtk requires GTK parent, Makepad uses raw X11) |
| | Cannot overlay Makepad widgets on top of the webview |

**Approach B: Offscreen Texture Rendering**

Run a browser engine that renders to a pixel buffer, pipe those pixels into a Makepad texture.

| Pros | Cons |
|------|------|
| Architecturally aligned with Makepad's design | Significant binary size increase |
| Makepad already has a texture pipeline | Performance overhead (offscreen render) |
| Makepad widgets can overlay on top | Complex input event translation |
| No Makepad platform changes needed | Substantial engineering effort |

#### 3d. Platform Isolation Analysis — Why Approach B Wins

A key architectural concern is **isolating platform-specific code**. Robrix targets Windows,
macOS, Linux, Android, iOS, and potentially web/WASM. Any solution should minimize the amount
of per-platform code Robrix itself has to maintain.

**Approach A (Child Window Overlay) leaks platform specifics at every layer:**
- Extracting the native handle is per-platform (`HWND`, `NSView`, `X11 Window`)
- Implementing `HasWindowHandle` in Makepad = platform-specific code in Makepad itself
- Child window positioning, Z-ordering, and resize coordination differ per OS
- Linux has a **fundamental incompatibility**: webkit2gtk requires a GTK parent, but Makepad
  uses raw X11
- Results in `#[cfg(target_os = ...)]` blocks scattered through the integration code

**Approach B (Offscreen Texture) has a clean platform-agnostic boundary:**
- The interface between Makepad and the browser engine is just two things:
  1. **Texture in** — rendered pixels from the engine flow into a Makepad texture
  2. **Events out** — Makepad input events (clicks, keys, scrolls) are forwarded to the engine
- Both are abstractions Makepad already handles cross-platform
- All platform specifics are **contained inside the browser engine**
- GPU texture sharing can be optimized per-platform but can always fall back to a simple CPU
  memcpy — a progressive optimization, not a hard requirement

The boundary looks like:
```
┌─────────────────────────────────┐
│  Makepad (platform-agnostic)    │
│  - Owns the widget area         │
│  - Displays texture             │
│  - Captures input events        │
└──────────┬──────────────────────┘
           │  Texture (pixels)
           │  Input events (clicks, keys, scroll)
           │  postMessage JSON strings
┌──────────┴──────────────────────┐
│  Browser Engine                 │
│  (owns its own platform details)│
│  - Renders HTML/CSS/JS          │
│  - Handles web standards        │
└─────────────────────────────────┘
```

**Conclusion:** Approach B is the recommended path. The rest of this research focuses on it.

#### 3e. How Other Custom-Renderer Frameworks Handle This

| Framework | Approach | Notes |
|-----------|----------|-------|
| **egui** | No built-in support. Experimental `egui_wry` uses child window overlay. | Maintainer says webview "is not expected to be part of egui proper anytime soon" |
| **iced** | `iced_webview` uses Ultralight for offscreen texture rendering. wry attempts "failed". | Offscreen renderers more tractable than system webviews |
| **Slint** | Servo integration via offscreen texture + GPU memory sharing. | Most complete reference. Substantial engineering effort. |

#### 3f. matrix-rust-sdk Already Implements the Widget API Protocol

**This is the most significant finding.** Robrix already depends on `matrix-sdk` (from git
main branch). The SDK has a comprehensive widget module at `crates/matrix-sdk/src/widget/`
gated behind the `experimental-widgets` feature flag.

**What the SDK provides (the entire protocol layer):**
- `WidgetDriver` + `WidgetDriverHandle` — full Widget API state machine
- `WidgetDriverHandle::send(json)` / `recv() -> json` — bidirectional message channel
- Capability negotiation, event forwarding, OpenID tokens, encryption, URL generation

**What the client must implement:**
1. **Webview embedding** — load a URL, support JS `postMessage`
2. **Message transport bridge** — route JSON between webview and `WidgetDriverHandle`
3. **CapabilitiesProvider** — decide which capabilities to grant
4. **Lifecycle management** — start/stop driver task, manage webview

**To enable:** Add `"experimental-widgets"` to `matrix-sdk` features in `app/Cargo.toml`.
The bridge layer would be ~100-200 lines of Rust.

---

#### 3g. Deep Dive: Approach B — Offscreen Texture Rendering

This section explains, in detail, how Approach B would work. It covers all the components
involved, what each one does, and how they differ across platforms.

##### What "Offscreen Texture Rendering" Means (Plain English)

Normally, a browser engine renders a webpage into a visible window on your screen. In our
case, Makepad already owns the entire window — it draws every pixel using its own GPU
rendering pipeline. We can't just plop a browser window inside it.

Instead, we ask the browser engine to render "offscreen" — meaning into a block of memory
(a pixel buffer) rather than directly to the screen. Then we take those pixels and hand them
to Makepad as a **texture** (like an image). Makepad draws that texture inside a widget area,
just like it would display any other image. The user sees the webpage embedded in the Robrix
UI, but under the hood, it's actually an image that gets refreshed many times per second.

For input (clicks, keyboard, scrolling), the flow is reversed: Makepad captures the user's
interactions on that widget area and forwards them to the browser engine, which processes
them as if the user had clicked/typed directly in a real browser.

##### The Components and Their Responsibilities

There are **five main components** in the system. Here is what each one does:

```
┌──────────────────────────────────────────────────────────────────────┐
│                         ROBRIX APPLICATION                          │
│                                                                     │
│  ┌─────────────────┐     ┌──────────────────┐    ┌──────────────┐  │
│  │  1. WebView      │     │  2. Widget API   │    │  3. Makepad  │  │
│  │     Widget        │◄───►│     Bridge       │◄──►│    Widget    │  │
│  │  (Makepad widget  │     │  (JSON router)   │    │   (texture   │  │
│  │   that displays   │     │                  │    │   display +  │  │
│  │   the texture)    │     │                  │    │   input      │  │
│  │                   │     │                  │    │   capture)   │  │
│  └────────┬──────────┘     └──────────────────┘    └──────────────┘  │
│           │                        ▲                                 │
│           │ pixels + events        │ JSON messages                   │
│           ▼                        ▼                                 │
│  ┌─────────────────┐     ┌──────────────────┐                       │
│  │  4. Browser      │     │  5. matrix-sdk   │                       │
│  │     Engine        │◄───►│    WidgetDriver  │                       │
│  │  (renders HTML)   │     │  (protocol       │                       │
│  │                   │     │   state machine) │                       │
│  └───────────────────┘     └──────────────────┘                       │
└──────────────────────────────────────────────────────────────────────┘
```

**Component 1 & 3: The Makepad WebView Widget**

This is a Makepad widget (like a button or text box) but instead of drawing text or shapes,
it draws a texture that contains the browser's rendered output. It also captures all mouse
clicks, keyboard presses, and scroll events that happen on its area and forwards them to
the browser engine.

Responsibilities:
- Create and own a Makepad `Texture` (a GPU image buffer)
- In its `draw_walk()` method, draw that texture to fill the widget area
- In its `handle_event()` method, intercept `Hit::FingerDown`, `Hit::FingerMove`,
  `Hit::KeyDown`, `Hit::TextInput`, etc. and forward them to the browser engine
- Request keyboard focus when clicked (so keyboard events go to this widget)
- Trigger a redraw whenever the browser produces a new frame
- Convert coordinates from Makepad's logical-pixel system to the browser engine's expected
  coordinate system (accounting for DPI/display scaling)

**Component 2: The Widget API Bridge**

This is the "glue" between the browser engine (which runs JavaScript) and the matrix-rust-sdk
`WidgetDriver` (which handles the Matrix protocol). Matrix widgets communicate with their host
using `window.postMessage()` — they send JSON messages and receive JSON messages back.

Responsibilities:
- Listen for messages coming OUT of the browser engine (JS calling `postMessage`)
- Forward those JSON strings to `WidgetDriverHandle::send(json)`
- Listen for messages coming FROM `WidgetDriverHandle::recv()`
- Inject those JSON strings back INTO the browser engine (via `evaluate_script` or similar)
- This is a thin layer — roughly 100-200 lines of Rust

**Component 4: The Browser Engine (Platform Backend)**

This is the actual web renderer — the thing that takes HTML, CSS, and JavaScript and turns
them into pixels. It runs "offscreen" and produces a pixel buffer that gets handed to the
Makepad widget as a texture.

Responsibilities:
- Load a URL and render the webpage
- Execute JavaScript (the widget's code)
- Produce pixel buffers (rendered frames) and send them to the Makepad widget
- Accept input events (mouse, keyboard, scroll) from the Makepad widget and process them
- Provide a way for JS code to send messages to the Rust host (for the Widget API bridge)
- Handle all platform-specific rendering details internally

**Important: Widgets are third-party content served from external widget servers.**

Matrix widgets are NOT local HTML that we control. They are third-party web applications
hosted on remote servers, loaded by URL. A widget expects to run inside an `<iframe>` and
communicates with its host via `window.parent.postMessage()`. This has several implications:

**For offscreen backends (desktop — Servo, CEF):**
The browser engine loads pages as top-level documents, not iframes. If we load the widget URL
directly, `window.parent === window` (there is no parent), and the widget's `postMessage`
calls go nowhere. The solution is a **host page pattern**: we load a small local HTML page
that contains an `<iframe src="WIDGET_URL">` and JS glue that bridges `postMessage` to the
Rust host:

```
Offscreen browser engine loads:
┌──────────────────────────────────────┐
│  Host page (local/injected HTML)     │
│                                      │
│  ┌────────────────────────────────┐  │
│  │  <iframe src="WIDGET_URL">    │  │
│  │  (third-party widget)         │  │
│  └────────────────────────────────┘  │
│                                      │
│  <script>                            │
│    // widget → Rust: intercept       │
│    window.addEventListener('message',│
│      e => hostBridge.send(e.data))   │
│                                      │
│    // Rust → widget: forward         │
│    hostBridge.onMessage(json =>      │
│      iframe.postMessage(json, '*'))  │
│  </script>                           │
└──────────────────────────────────────┘
```

This also provides **iframe sandboxing for free** — the third-party widget runs in a
cross-origin iframe with the browser's standard same-origin policy, CSP, and security
isolation. This matters because the widget code is untrusted.

**For overlay backends (mobile — WKWebView, Android WebView):**
Element X already solves this. The matrix-rust-sdk `WidgetDriver` generates a URL with
parameters that tell the widget JS SDK which transport to use. On native platforms, the
widget uses the native JS bridge (`WKScriptMessageHandler` on iOS, `addJavascriptInterface`
on Android) instead of `postMessage`. No iframe host page is needed.

**For WASM (iframe backend):**
The widget is loaded in a real `<iframe>` inside the browser. `postMessage` works natively
between the Makepad WASM app and the iframe. This is the simplest case.

**Other implications of third-party content:**
- The browser engine needs full **network access** (HTTPS, fetch, WebSocket, CORS)
- It needs proper **TLS/certificate handling** for secure connections
- It needs **cookie and storage** support (widgets may use localStorage, sessionStorage)
- Servo's incomplete web standards become a **bigger risk** — since we don't control the
  widget code, we can't work around missing APIs. Widgets are typically React/Vue SPAs that
  rely on modern web platform features. CEF (full Chromium) is the safer choice for web
  compatibility, though it comes with much larger binary size.

**Component 5: matrix-rust-sdk WidgetDriver**

This already exists and handles the Matrix Widget API protocol. We don't need to build it.

Responsibilities:
- Manage the Widget API state machine (capability negotiation, message routing)
- Read/send Matrix room events on behalf of the widget
- Handle OpenID token requests, delayed events, to-device messages
- Expose a simple `send(json)` / `recv() -> json` interface

##### How Makepad's Texture System Works (Technical Detail)

Makepad has a texture system that lets you push raw pixel data to the GPU. Here's how it
works concretely:

**Texture format for our use case: `TextureFormat::VecBGRAu8_32`**

This stores pixels as a `Vec<u32>` where each `u32` is one pixel packed as:
`(alpha << 24) | (red << 16) | (green << 8) | blue`

So for a 1920x1080 widget, the buffer is `1920 * 1080 = 2,073,600` u32 values (~8 MB).

**Creating and updating a texture:**
```
1. Create:  Texture::new_with_format(cx, TextureFormat::VecBGRAu8_32 { ... })
2. Update:  texture.take_vec_u32(cx)    -- takes the buffer out (zero-copy swap)
            ... modify pixels ...
            texture.put_back_vec_u32(cx, data, dirty_rect)  -- puts it back
3. Display: draw_bg.draw_vars.set_texture(0, &texture)      -- bind to shader
            draw_bg.draw_walk(cx, walk)                      -- draw the quad
```

The GPU upload happens lazily during the next draw pass — Makepad detects that the texture
was marked as updated and uploads it to the GPU before drawing.

**Thread safety:** Textures must be updated on the main (UI) thread. The browser engine will
typically run on its own background thread. Makepad provides `Cx::post_action()` — a
thread-safe way to send data from any thread to the main thread. The pattern is:
1. Browser engine renders a frame on its background thread
2. It calls `Cx::post_action(BrowserFrame { pixels, width, height })`
3. Makepad's main thread receives this as an action in the next event cycle
4. The widget swaps the pixel data into the texture and calls `self.redraw(cx)`

**Makepad's rendering backends per platform:**

| Platform | GPU API | Texture upload method |
|----------|---------|----------------------|
| Windows | Direct3D 11 | `ID3D11Texture2D` with `DXGI_FORMAT_B8G8R8A8_UNORM` |
| macOS | Metal | `MTLTexture` with `BGRA8Unorm`, `replaceRegion` |
| iOS | Metal | Same as macOS (shared code) |
| Linux | OpenGL (EGL) | `glTexImage2D` / `glTexSubImage2D` |
| Android | OpenGL ES (EGL) | Same as Linux (shared code) |
| Web/WASM | WebGL | Managed by JS side |

Note: Makepad does NOT use wgpu — it has its own custom rendering layer that talks directly
to each platform's GPU API.

##### How Makepad's Input System Works (Technical Detail)

When the user clicks, types, or scrolls inside a widget, Makepad delivers events through
a "hits" system. In a widget's `handle_event()` method, you call `event.hits(cx, area)` and
get back a `Hit` enum telling you what happened:

- `Hit::FingerDown` — mouse click or touch start. Includes `abs` (position in logical
  pixels), `digit_id` (which finger/button), `tap_count` (for double-click detection)
- `Hit::FingerMove` — mouse drag or touch move
- `Hit::FingerUp` — mouse release or touch end
- `Hit::FingerScroll` — scroll wheel or two-finger scroll. Includes `scroll: DVec2` (delta)
- `Hit::FingerHoverOver` — mouse hover (no button pressed)
- `Hit::KeyDown` / `Hit::KeyUp` — keyboard events. Include `key_code` and `modifiers`
- `Hit::TextInput` — actual character input (handles IME, dead keys, etc.)

All coordinates are in **logical (DPI-aware) pixels**. To get physical pixels (which some
browser engines expect), multiply by `cx.get_dpi_factor_of(&area)`.

To get widget-local coordinates: `local_pos = abs - rect.pos` (where `rect` is the widget's
clipped rectangle, provided in the event).

To receive keyboard events, the widget must first call `cx.set_key_focus(area)` (typically
on `FingerDown`) to claim keyboard focus.

##### Browser Engine Options for Offscreen Rendering

Not all browser engines can render "offscreen." Here's what works and what doesn't:

**Servo (Rust-native browser engine) — Best long-term option**

Servo is a browser engine written entirely in Rust. It uses Mozilla's SpiderMonkey for
JavaScript and its own layout/rendering engine.

- **Offscreen rendering:** First-class support via `OffscreenRenderingContext`. Can render
  to a framebuffer and extract pixels via `read_to_image()` (returns RGBA `ImageBuffer`).
- **GPU texture sharing (zero-copy, avoids CPU copies):**
  - macOS: IOSurface — shared GPU memory that both Servo and Metal can access
  - Linux: `VK_KHR_external_memory` + `GL_EXT_memory_object_fd` — Vulkan/OpenGL interop
  - Windows: Software rendering currently; DirectX path in progress
- **JS ↔ Rust:** `evaluate_javascript()` for Rust→JS. No built-in JS→Rust callback channel
  yet — would need a custom bridge (e.g., intercepting resource requests, or polling via JS
  eval).
- **Web standards:** SpiderMonkey handles modern JS well. Shadow DOM supported. CSS Grid
  partially supported. WebRTC likely NOT supported. A typical React/Vue SPA may or may not
  work depending on which APIs it uses.
- **Platforms:** Windows, macOS, Linux, Android (experimental). **No iOS, no WASM.**
- **Binary size:** ~100-150 MB (includes SpiderMonkey, WebRender, full layout engine)
- **Maturity:** Rapidly evolving but NOT production-ready. The embedding API only stabilized
  in early 2025 and is still changing. The Slint integration proves it works, but you should
  expect rough edges.

**CEF (Chromium Embedded Framework) — Best web compatibility**

CEF wraps the full Chromium browser engine. It's what powers Electron, Spotify, and Steam.

- **Offscreen rendering (OSR):** Mature, well-documented.
  - `OnPaint()` — delivers BGRA8 pixel buffers with dirty rectangles (software path)
  - `OnAcceleratedPaint()` — delivers D3D11 shared texture handles (Windows only, zero-copy)
- **JS ↔ Rust:** `ExecuteJavaScript()` for host→JS. `CefMessageRouter` for JS→host
  (purpose-built async message passing). Full Chromium `postMessage` works.
- **Rust crates:** `wew` (Windows/macOS/Linux X11, includes `MessageTransport` for
  bidirectional async JS↔Rust communication) and `wef` (experimental).
- **Web standards:** Full Chromium compliance. Everything works — React, Vue, WebRTC, all of it.
- **Platforms:** Windows, macOS, Linux. **No Android, no iOS, no WASM.**
- **Binary size:** ~93-160 MB (DLLs, resource packs, locales, subprocess executables).
  Total app distribution can reach ~1 GB.
- **Build complexity:** Cannot use standard Cargo workflows. CEF requires multi-process
  architecture with helper executables, resource files, and platform-specific packaging.
- **Maturity:** Battle-tested. Rust bindings (wew, wef) are experimental but the underlying
  CEF is rock-solid.

**System Webviews (WebView2, WKWebView, Android WebView) — NOT viable for offscreen**

None of the OS-provided webviews were designed for offscreen rendering:

| Platform | Engine | Offscreen? | Notes |
|----------|--------|-----------|-------|
| Windows | WebView2 | Hacky workaround only | No official API. Workaround uses Graphics Capture API. |
| macOS | WKWebView | Snapshots only | `takeSnapshot()` is too slow for continuous rendering. Requires view hierarchy. |
| iOS | WKWebView | No | Must be in the view hierarchy to render at all. |
| Android | Android WebView | Partially | `draw(canvas)` works but loses hardware-accelerated content (WebGL, video). |
| Linux | WebKitGTK | Requires GTK | Can work via `gtk_offscreen_window_new()` but pulls in all of GTK. |

**Verdict:** System webviews are not viable for the offscreen texture approach. They were
designed to be visible UI elements, not headless renderers.

##### Per-Platform Strategy — What To Use Where

Because no single engine covers all platforms, we need a **platform adapter** pattern:
define a common trait and provide different implementations per platform.

```
┌────────────────────────────────────────────────────────────────┐
│                    Robrix Widget Code                          │
│               (platform-agnostic Rust)                        │
│                                                                │
│    Uses a WebViewBackend trait — doesn't know or care          │
│    which engine is behind it                                   │
└──────────────────────┬─────────────────────────────────────────┘
                       │
        ┌──────────────┼──────────────────────┐
        │              │                      │
        ▼              ▼                      ▼
  ┌───────────┐  ┌───────────┐        ┌─────────────┐
  │  Offscreen │  │  Overlay  │        │  iframe     │
  │  Backend   │  │  Backend  │        │  Backend    │
  │ (Desktop)  │  │ (Mobile)  │        │  (WASM)     │
  │            │  │           │        │             │
  │ Servo/CEF  │  │ WKWebView │        │ HTML iframe │
  │ renders to │  │ or Android│        │ positioned  │
  │ pixel buf  │  │ WebView   │        │ over canvas │
  │ → texture  │  │ as native │        │             │
  └───────────┘  │ overlay   │        └─────────────┘
                  └───────────┘
```

Here is the strategy for each platform:

**Windows — Offscreen via Servo (or CEF fallback)**

- Servo renders to an offscreen framebuffer
- Currently software rendering (CPU buffer → Makepad texture via `VecBGRAu8_32`)
- Hardware DirectX path is in development — once ready, could use D3D11 shared textures
- CEF is a proven fallback with `OnAcceleratedPaint()` for zero-copy D3D11 textures
- Binary size: ~100 MB (Servo) or ~93 MB (CEF)
- Rust integration: Direct crate (Servo) or `wew` crate (CEF)

**macOS — Offscreen via Servo**

- Best-supported platform for Servo embedding (proven in Slint)
- Zero-copy GPU texture sharing via IOSurface:
  Servo renders via OpenGL → IOSurface → imported as MTLTexture → Makepad draws it
- Binary size: ~105 MB
- Rust integration: Direct crate + `objc2-metal` for IOSurface/Metal interop

**Linux — Offscreen via Servo (or CEF fallback)**

- Servo with Vulkan/OpenGL texture sharing:
  Servo renders via OpenGL → exports fd via `GL_EXT_memory_object_fd` →
  Makepad imports as OpenGL texture
- This avoids any GTK dependency (unlike WebKitGTK)
- CEF is a proven fallback with `OnPaint()` software path
- Binary size: ~100 MB
- Rust integration: Direct crate (Servo) or `wew` crate (CEF)

**Android — Native Overlay (system WebView)**

- Offscreen rendering with Android WebView is unreliable (loses WebGL, hardware content)
- Servo has Android support but it's experimental
- The practical approach (used by Element X Android): use the system Android WebView as a
  regular native View, positioned on top of the Makepad rendering surface
- This means the webview is NOT inside Makepad's texture system — Android's compositor
  layers it on top. This has z-ordering limitations (webview always on top).
- Zero binary size cost (system-provided)
- Requires JNI (Java Native Interface) bridge from Rust to Android APIs
- The Makepad widget tracks position/size and communicates it to the Java side

**iOS — Native Overlay (WKWebView)**

- WKWebView is the ONLY option (Apple mandates WebKit for all web content on iOS)
- Offscreen rendering is not supported — WKWebView must be in the view hierarchy
- Same approach as Android: overlay a real WKWebView UIView on top of Makepad's Metal surface
- iOS's compositor handles the layering
- Z-ordering limitation: webview always on top of Makepad content
- Zero binary size cost
- Requires Objective-C FFI (via `objc2` crate) to create and manage the WKWebView
- JS communication: `evaluateJavaScript()` for Rust→JS, `WKScriptMessageHandler` for JS→Rust

**Web/WASM — DOM iframe Overlay**

- This is the dramatically simpler case: the host IS already a browser!
- Makepad renders to an HTML5 `<canvas>` via WebGL
- We create a regular `<iframe>` element and position it on top of the canvas using CSS
  (`position: absolute`, matching the widget's coordinates)
- The browser's own compositor handles layering
- No pixel copying, no offscreen rendering, no browser engine to embed
- `postMessage` works natively between the Makepad WASM app and the iframe
- Z-ordering limitation: iframe is always on top of the canvas
- Zero binary size cost
- Requires `wasm-bindgen` / `web_sys` for DOM manipulation from Rust

##### Summary: Per-Platform Recommendation

| Platform | Strategy | Engine | Rendering Mode | Size Impact | JS↔Rust | Maturity |
|----------|----------|--------|---------------|-------------|---------|----------|
| **Windows** | Offscreen texture | Servo (or CEF) | GPU framebuffer / D3D11 | ~100 MB | eval_js + custom bridge | Medium |
| **macOS** | Offscreen texture | Servo | IOSurface zero-copy | ~105 MB | eval_js + custom bridge | High (Slint-proven) |
| **Linux** | Offscreen texture | Servo (or CEF) | Vulkan/GL shared texture | ~100 MB | eval_js + custom bridge | High (Slint-proven) |
| **Android** | Native overlay | System WebView | OS-composited overlay | 0 MB | `addJavascriptInterface` | Medium (JNI needed) |
| **iOS** | Native overlay | WKWebView | OS-composited overlay | 0 MB | `WKScriptMessageHandler` | Medium (only option) |
| **Web/WASM** | DOM iframe | Browser | CSS-positioned iframe | 0 MB | `window.postMessage` | High (trivial) |

##### The Platform Adapter Trait

To keep Robrix's core code platform-agnostic, we define a trait that each platform implements:

```rust
/// What the Robrix widget code calls — platform-agnostic interface.
trait WebViewBackend {
    /// Load a URL in the webview
    fn load_url(&mut self, url: &str);

    /// Execute JavaScript in the webview (Rust → JS)
    fn evaluate_js(&mut self, script: &str);

    /// Register a callback for messages from JS (JS → Rust)
    fn on_message(&mut self, callback: Box<dyn Fn(String)>);

    // --- For OFFSCREEN backends (desktop) ---

    /// Get the latest rendered frame as a pixel buffer
    fn take_frame(&mut self) -> Option<FrameBuffer>;

    /// Forward a mouse/touch event to the browser engine
    fn send_input_event(&mut self, event: InputEvent);

    // --- For OVERLAY backends (mobile, WASM) ---

    /// Update the native view's position and size to match the Makepad widget
    fn set_bounds(&mut self, rect: Rect);

    /// Show or hide the native view
    fn set_visible(&mut self, visible: bool);
}
```

There are two categories of implementation:
1. **Offscreen backends** (Servo, CEF) — render to a pixel buffer. Makepad forwards input
   events. Full z-ordering control.
2. **Overlay backends** (WKWebView, Android WebView, iframe) — a native element is positioned
   on top of Makepad's rendering surface. The browser handles its own input. Always on top.

---

### PoC: Show Web Content in Robrix (Windows + CEF)

**Status:** Complete (branch `taj/minimal-widget-poc`)

**Goal:** A minimal proof-of-concept that loads a webpage and renders it as a texture inside
a Makepad widget in Robrix, on Windows. No Matrix Widget API, no postMessage, no input
forwarding — just: "can we see a webpage inside Robrix?"

#### What "Success" Looks Like

The PoC replaces the welcome/home panel with a full-panel `WebViewWidget` that renders
`https://wid.staging.verji.app/` via CEF. The browser renders at the widget's actual pixel
dimensions (dynamically resized) and the content is displayed as a GPU texture. No
interaction (clicks/keyboard) — that's a follow-up step.

#### Which CEF Crate?

Three Rust crates wrap CEF. Here's the quick comparison for the PoC:

| Crate | Pros | Cons |
|-------|------|------|
| **wew** | Highest-level API. Auto-downloads CEF. Built-in `MessageTransport` for JS comms. | Not on crates.io (git dep). No `evaluate_javascript()`. |
| **cef-rs** | Tauri-maintained. On crates.io. Full CEF API. Accelerated OSR (D3D11 shared textures). | Lower-level — more boilerplate (~350 lines for basic OSR). |
| **wef** | Cleanest Rust API. Has `execute_javascript()`. On crates.io. | Experimental/paused. Authors switched to wry for production. |

**For the PoC: use `wew`** — it's the fastest path to getting pixels. Its high-level API
and automatic CEF download mean less setup. The lack of `evaluate_javascript()` doesn't
matter for the PoC (we just want to display a page). For production, we'd likely switch to
`cef-rs`.

#### The Moving Parts (Explained Simply)

Here's what happens when the PoC runs, step by step:

```
  Robrix starts up
       │
       ▼
  ┌─────────────────────────────────────────────────────────┐
  │ 1. CEF Runtime initializes                              │
  │    - wew downloads/loads CEF libraries (libcef.dll etc) │
  │    - Spawns helper subprocess (needed by Chromium)      │
  │    - Sets up offscreen rendering mode                   │
  └────────────────────┬────────────────────────────────────┘
                       │
                       ▼
  ┌─────────────────────────────────────────────────────────┐
  │ 2. WebViewWidget detects its layout size                 │
  │    - Reports actual pixel dimensions via set_desired_size│
  │    - CefManager picks up the size on next poll()         │
  └────────────────────┬────────────────────────────────────┘
                       │
                       ▼
  ┌─────────────────────────────────────────────────────────┐
  │ 3. Create an offscreen browser at the widget's size      │
  │    - Loads the configured URL (e.g. wid.staging.verji)  │
  │    - Browser is invisible — no window, just memory      │
  │    - Resizes dynamically when widget size changes        │
  └────────────────────┬────────────────────────────────────┘
                       │
                       ▼
  ┌─────────────────────────────────────────────────────────┐
  │ 4. CEF renders the page and calls on_frame()            │
  │    - Delivers BGRA8 pixel buffer (W * H * 4 bytes)      │
  │    - Called every time the page content changes          │
  │    - Pixels stashed in a global Mutex, Makepad signalled │
  └────────────────────┬────────────────────────────────────┘
                       │
                       ▼
  ┌─────────────────────────────────────────────────────────┐
  │ 5. Makepad widget receives the pixels                   │
  │    - Swaps them into a VecBGRAu8_32 texture             │
  │    - Recreates texture if frame dimensions changed       │
  │    - Draws the texture via shader in the widget area     │
  │    - User sees the webpage inside Robrix!                │
  └─────────────────────────────────────────────────────────┘
```

#### Components To Build

There are **four things** we need to create:

**A. The CEF helper binary** (`robrix-cef-helper.exe`)

CEF uses a multi-process architecture (Chromium does too — that's why Chrome has many
processes in Task Manager). The helper is a tiny separate executable that CEF launches
automatically for its renderer/GPU/utility processes.

```
File: app/src/bin/robrix_cef_helper.rs   (or a separate crate)

Entire contents:
    fn main() {
        wew::execute_subprocess();
    }
```

That's it — one line. wew handles the rest internally.

**B. The CEF manager** (initialization + lifecycle)

File: `app/src/webview/cef_manager.rs`

This module starts up CEF when Robrix launches and owns the runtime and message loop.
It also handles dynamic resize: the widget reports its layout size via global atomics
(`DESIRED_WIDTH`/`DESIRED_HEIGHT`), and `poll()` calls `webview.resize()` when they change.

Responsibilities:
- Create CEF's message loop in "pump" mode (`MessagePumpLoop`) so Robrix controls polling
- Create the CEF runtime with settings (cache path, log level, path to helper exe)
- Provide a `poll()` method called from Makepad's event loop to drive CEF
- Create the offscreen browser once CEF signals readiness (via `CEF_INITIALIZED` flag)
- Resize the browser when the widget's layout size changes

Frame delivery uses a global `Mutex<Option<FrameData>>` (`LATEST_FRAME`) — CEF's
`on_frame` callback writes to it, the widget reads from it. `SignalToUI::set_ui_signal()`
wakes Makepad after each frame or when CEF needs pump work.

```rust
// Actual implementation (simplified):
pub struct CefManager {
    message_loop: MessagePumpLoop,
    runtime: Runtime<MessagePumpLoop, WindowlessRenderWebView>,
    webview: Option<WebView<WindowlessRenderWebView>>,
    browser_url: String,
    browser_width: u32,
    browser_height: u32,
}

impl CefManager {
    pub fn new(url: &str, width: u32, height: u32) -> Result<Self, wew::Error> { ... }

    pub fn poll(&mut self) {
        self.message_loop.poll();
        // Create browser once CEF is initialized
        if self.webview.is_none() && CEF_INITIALIZED.load(Ordering::Acquire) {
            // Use desired size from widget if available
            self.create_browser();
        }
        // Resize browser if widget reported a new size
        if let Some(webview) = &self.webview {
            let (dw, dh) = (DESIRED_WIDTH.load(..), DESIRED_HEIGHT.load(..));
            if dw != self.browser_width || dh != self.browser_height {
                webview.resize(dw, dh);
            }
        }
    }
}
```

**C. The frame receiver** (CEF → pixel buffer → Makepad)

This is the `WebViewObserver` callback that CEF calls every time it has new pixels. It writes
to a global `Mutex<Option<FrameData>>` and wakes Makepad via `SignalToUI::set_ui_signal()`.

The original plan used `Cx::post_action()` but the actual implementation uses a simpler
global mutex approach — CEF writes, the widget reads on the next event cycle.

```rust
// Actual implementation:
impl WindowlessRenderWebViewHandler for WebViewObserver {
    fn on_frame(&self, frame: &Frame) {
        if !matches!(frame.ty, FrameType::View) { return; }

        // CEF delivers BGRA8 bytes. On little-endian, from_ne_bytes gives
        // exactly the layout Makepad's VecBGRAu8_32 expects.
        let pixels: Vec<u32> = frame.buffer
            .chunks_exact(4)
            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        if let Ok(mut latest) = LATEST_FRAME.lock() {
            *latest = Some(FrameData { pixels, width: frame.width, height: frame.height });
        }
        SignalToUI::set_ui_signal();
    }
}
```

**D. The Makepad widget** (`WebViewWidget`)

File: `app/src/shared/web_view_widget.rs`

A Makepad widget that displays CEF frames as a GPU texture. It detects its own layout size
after `draw_walk` and reports it to CEF via `set_desired_size()`. On each event cycle it
checks for new frames and updates the texture.

Key implementation details:
- Tracks texture dimensions (`tex_width`/`tex_height`) separately from widget dimensions
  to avoid garbled rendering when sizes change during resize
- Only uses `swap_vec_u32` when frame dimensions match the texture exactly; recreates the
  texture otherwise
- Uses `self.view.area().rect(cx)` after `draw_walk` to detect actual layout size

```rust
// Actual implementation (simplified):
live_design! {
    pub WebViewWidget = {{WebViewWidget}} {
        width: Fill, height: Fill,
        show_bg: true,
        draw_bg: {
            texture tex: texture2d
            fn pixel(self) -> vec4 {
                return sample2d(self.tex, self.pos).xyzw;
            }
        }
    }
}

#[derive(Live, LiveHook, Widget)]
pub struct WebViewWidget {
    #[deref] view: View,
    #[rust] texture: Option<Texture>,
    #[rust] tex_width: usize,   // track texture dimensions
    #[rust] tex_height: usize,  // separately from widget size
    #[rust] last_widget_width: u32,
    #[rust] last_widget_height: u32,
}

impl Widget for WebViewWidget {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
        if let Some(frame) = crate::webview::take_latest_frame() {
            self.update_texture(cx, frame);
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let result = self.view.draw_walk(cx, scope, walk);
        // Report actual layout size to CEF for dynamic resize
        let rect = self.view.area().rect(cx);
        let (w, h) = (rect.size.x as u32, rect.size.y as u32);
        if w > 0 && h > 0 && (w != self.last_widget_width || h != self.last_widget_height) {
            self.last_widget_width = w;
            self.last_widget_height = h;
            crate::webview::set_desired_size(w, h);
        }
        result
    }
}
```

#### How It Fits Into the Robrix Codebase

```
app/
├── Cargo.toml                          # Add wew dependency
├── src/
│   ├── bin/
│   │   └── robrix_cef_helper.rs        # NEW — CEF subprocess helper (1 line)
│   ├── webview/                        # NEW — webview module
│   │   ├── mod.rs                      # Module declarations
│   │   └── cef_manager.rs             # CEF initialization & lifecycle
│   ├── shared/
│   │   ├── mod.rs                      # Add web_view_widget to live_design()
│   │   └── web_view_widget.rs          # NEW — Makepad widget that shows the texture
│   ├── app.rs                          # Wire up: init CefManager, add widget to UI
│   └── lib.rs                          # Add webview module
```

Changes to existing files:
- `app/Cargo.toml` — added `wew = { git = "https://github.com/mycrl/wew" }` + `[[bin]]` for helper
- `app/src/lib.rs` — added `pub mod webview;`
- `app/src/shared/mod.rs` — added `pub mod web_view_widget;` and registered in `live_design()`
- `app/src/app.rs` — added `CefManager` field, init on startup, `poll()` on every event cycle
- `app/src/home/welcome_screen.rs` — replaced welcome text with full-panel `WebViewWidget`

#### Build Prerequisites (Windows)

The `wew` crate's build script compiles CEF's C++ wrapper and generates Rust bindings,
which requires:

- **LLVM/Clang** — `winget install LLVM.LLVM` then set `LIBCLANG_PATH="C:/Program Files/LLVM/bin"`
- **CMake** — `winget install Kitware.CMake` and add to PATH

First build takes ~9 minutes (downloads CEF, compiles C++ wrapper, builds all deps).
Incremental rebuilds (Rust-only changes) take ~3 minutes.

#### CEF Runtime Files (Post-Build Step)

After `cargo build`, CEF's DLLs and resources need to be copied next to the Robrix binary.
wew downloads them during build into the build output directory. Copy them alongside
`robrix.exe`:

```bash
# Source: app/target/release/build/wew-<hash>/out/cef/
CEF_DIR="app/target/release/build/wew-*/out/cef"

# Copy Release DLLs + data files
cp $CEF_DIR/Release/*.dll app/target/release/
cp $CEF_DIR/Release/v8_context_snapshot.bin app/target/release/
cp $CEF_DIR/Release/vk_swiftshader_icd.json app/target/release/

# Copy Resources
cp $CEF_DIR/Resources/icudtl.dat app/target/release/
cp $CEF_DIR/Resources/resources.pak app/target/release/
cp $CEF_DIR/Resources/chrome_*.pak app/target/release/
mkdir -p app/target/release/locales
cp $CEF_DIR/Resources/locales/*.pak app/target/release/locales/
```

Result:
```
app/target/release/
├── robrix.exe
├── robrix-cef-helper.exe
├── libcef.dll, chrome_elf.dll, d3dcompiler_47.dll, dxil.dll,
│   dxcompiler.dll, libEGL.dll, libGLESv2.dll, vk_swiftshader.dll, vulkan-1.dll
├── v8_context_snapshot.bin, vk_swiftshader_icd.json
├── icudtl.dat, resources.pak, chrome_100_percent.pak, chrome_200_percent.pak
└── locales/
    └── en-US.pak  (+ 54 other locales)
```

#### CEF Event Loop Integration

The trickiest part is making CEF and Makepad share the main thread. Both want to run an
event loop. The solution: CEF in "MessagePump" mode, where WE control when it runs.

```
Makepad event loop (main thread):
    loop {
        wait_for_event()
        ├── handle Makepad events (draw, input, actions)
        ├── cef_manager.poll()     ◄── give CEF a chance to process its work
        └── repeat
    }
```

CEF's `MessagePumpRuntimeHandler` callback tells us WHEN it needs `poll()` called via
`on_schedule_message_pump_work(delay_ms)`. We could use this to request a Makepad
redraw/wake after the specified delay, ensuring CEF gets timely processing without busy-
looping.

#### What This PoC Does NOT Include (Follow-Up Steps)

This PoC proves that we can get CEF pixels onto a Makepad texture with dynamic resize.
The following would come in subsequent iterations:

1. **Mouse/keyboard input forwarding** — intercept `Hit::FingerDown/Move/Up/Scroll` and
   `Hit::KeyDown/TextInput` in the widget and forward to CEF via `webview.mouse()` /
   `webview.keyboard()`. wew's `EventAdapter` helps with this, though it expects winit events
   so we'd need to translate Makepad events to the wew format.
2. **JS ↔ Rust communication** — use wew's `MessageTransport` to bridge `postMessage` for
   the Matrix Widget API.
3. **Host page with iframe** — load the widget URL in an iframe inside a local host page (as
   described in section 3g) for proper `postMessage` routing and security sandboxing.
4. **matrix-rust-sdk WidgetDriver integration** — wire up `WidgetDriverHandle::send()`/`recv()`
   to the JS bridge.
5. **Widget lifecycle** — creating/destroying browser instances as widgets open/close.

**Completed in the PoC:**
- ~~Resize handling~~ — `webview.resize()` is called dynamically when the widget size changes.

---

### 4. Architecture & Complexity Assessment
**Status:** Not started
**Goal:** Sketch a possible architecture and estimate effort.

Questions to answer:
- What would a minimal viable widget host look like in Robrix?
- What are the hard problems?
- How would the widget UI integrate with the Robrix room view?
- What's the rough complexity? (Small / Medium / Large effort)
- What would a phased approach look like? (MVP -> full support)

Findings:
> _(to be filled in)_

---

## Summary & Recommendation

> _(to be filled in after research is complete)_

---

## References

### Matrix Widget API / SDK
- matrix-rust-sdk widget module: https://github.com/matrix-org/matrix-rust-sdk/tree/main/crates/matrix-sdk/src/widget
- matrix-rust-sdk widget docs: https://matrix-org.github.io/matrix-rust-sdk/matrix_sdk/widget/index.html
- Element X iOS architecture: https://deepwiki.com/element-hq/element-x-ios
- Element X Android: https://github.com/element-hq/element-x-android

### Browser Engines
- Servo project: https://github.com/servo/servo
- Servo embedding API docs: https://doc.servo.org/servo/struct.WebView.html
- Servo OffscreenRenderingContext: https://doc.servo.org/servo/struct.OffscreenRenderingContext.html
- Servo evaluate_javascript PR: https://github.com/servo/servo/pull/35720
- Slint + Servo integration: https://slint.dev/blog/using-servo-with-slint
- CEF OSR / OnAcceleratedPaint: https://github.com/chromiumembedded/cef/issues/3730
- CEF CefRenderHandler API: https://magpcss.org/ceforum/apidocs3/projects/(default)/CefRenderHandler.html
- wew (CEF Rust): https://github.com/mycrl/wew
- wef (CEF Rust): https://github.com/longbridge/wef

### Webview Crates
- wry: https://github.com/tauri-apps/wry
- cef-rs: https://github.com/tauri-apps/cef-rs
- webview2-com: https://github.com/wravery/webview2-rs
- webkit2gtk: https://github.com/tauri-apps/webkit2gtk-rs

### Platform-Specific References
- WebView2 offscreen rendering issue: https://github.com/MicrosoftEdge/WebView2Feedback/issues/547
- WKWebView offscreen rendering (Apple Forums): https://developer.apple.com/forums/thread/710015
- Android WebView to OpenGL texture: https://anuraagsridhar.wordpress.com/2013/03/13/rendering-an-android-webview-or-for-that-matter-any-android-view-directly-to-opengl/
- CEF Android support issue: https://bitbucket.org/chromiumembedded/cef/issues/1991/add-android-support

### Other Framework Integrations
- wry in Godot: https://github.com/tauri-apps/wry/issues/1335
- wry child webview discussion: https://github.com/tauri-apps/wry/issues/677
- egui webview discussion: https://github.com/emilk/egui/discussions/1353
- iced webview discussion: https://discourse.iced.rs/t/how-to-open-browser-inside-iced-app/549
