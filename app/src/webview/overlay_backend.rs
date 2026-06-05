//! Overlay backend: positions a native CEF child window over a Makepad widget.
//!
//! Uses `NativeWindowWebView` mode — CEF creates a real window that the OS
//! compositor renders and routes input to directly. No texture pipeline,
//! no manual mouse/keyboard forwarding.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering},
};
use std::time::Instant;

use makepad_widgets::SignalToUI;
use wew::{
    MessageLoopAbstract, MessagePumpLoop, NativeWindowWebView,
    runtime::{LogLevel, MessagePumpRuntimeHandler, Runtime, RuntimeHandler},
    webview::{WebView, WebViewAttributesBuilder, WebViewHandler, WebViewState},
};

use super::platform_windows;

// ---------------------------------------------------------------------------
// Widget → backend communication via atomics
// ---------------------------------------------------------------------------

static OVERLAY_X: AtomicI32 = AtomicI32::new(0);
static OVERLAY_Y: AtomicI32 = AtomicI32::new(0);
static OVERLAY_W: AtomicU32 = AtomicU32::new(0);
static OVERLAY_H: AtomicU32 = AtomicU32::new(0);

/// Incremented by the widget on each `draw_walk`. The backend compares this
/// to its last-seen value — if unchanged across several polls, the widget is
/// no longer being drawn (e.g. another tab is active) and we hide the window.
static OVERLAY_DRAW_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Called by the `WebViewOverlay` widget to report its layout rect (logical pixels).
pub fn set_overlay_rect(x: i32, y: i32, w: u32, h: u32) {
    OVERLAY_X.store(x, Ordering::Relaxed);
    OVERLAY_Y.store(y, Ordering::Relaxed);
    OVERLAY_W.store(w, Ordering::Relaxed);
    OVERLAY_H.store(h, Ordering::Relaxed);
    OVERLAY_DRAW_COUNTER.fetch_add(1, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Global backend instance (module-level, avoids #[cfg] on App struct fields)
// ---------------------------------------------------------------------------

static BACKEND: Mutex<Option<OverlayBackend>> = Mutex::new(None);

/// Write a WebRTC/video capability test page and return its `file://` URL.
pub fn write_webrtc_test_page() -> String {
    let dir = crate::app_data_dir().join("cef_cache");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join("webrtc_test.html");

    let html = r#"<!DOCTYPE html>
<html>
<head>
<style>
  body { margin: 0; padding: 20px; background: #1a1a2e; color: #eee;
         font-family: -apple-system, sans-serif; }
  h2 { color: #0ff; margin-top: 24px; }
  .ok { color: #0f0; } .fail { color: #f44; } .warn { color: #fa0; }
  video { width: 480px; height: 270px; background: #000; display: block;
          margin: 10px 0; border: 1px solid #444; }
  #cam { width: 320px; height: 240px; }
  button { padding: 8px 16px; margin: 4px; font-size: 14px; cursor: pointer;
           background: #0ff; border: none; color: #000; border-radius: 4px; }
  #log { white-space: pre-wrap; font: 13px monospace; color: #aaa;
         max-height: 200px; overflow-y: auto; margin-top: 8px;
         background: #111; padding: 8px; border-radius: 4px; }
</style>
</head>
<body>
<h1>CEF Overlay — WebRTC & Video Test</h1>

<h2>1. Browser Capabilities</h2>
<div id="caps"></div>

<h2>2. HTML5 Video (MP4)</h2>
<video id="v" controls autoplay muted loop playsinline>
  <source src="https://www.w3schools.com/html/mov_bbb.mp4" type="video/mp4">
</video>
<div id="vstatus">Loading...</div>

<h2>3. Camera / Microphone (getUserMedia)</h2>
<button onclick="testCamera()">Request Camera</button>
<button onclick="testMic()">Request Microphone</button>
<video id="cam" autoplay muted playsinline></video>
<div id="camstatus">Not tested yet</div>

<h2>4. WebRTC Peer Connection</h2>
<button onclick="testPeerConnection()">Test RTCPeerConnection</button>
<div id="pcstatus">Not tested yet</div>

<div id="log"></div>

<script>
var log = document.getElementById('log');
function L(msg) { log.textContent += msg + '\n'; log.scrollTop = log.scrollHeight; }

// 1. Capabilities
var caps = document.getElementById('caps');
var checks = [
  ['RTCPeerConnection', !!window.RTCPeerConnection],
  ['getUserMedia', !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia)],
  ['MediaStream', !!window.MediaStream],
  ['WebSocket', !!window.WebSocket],
  ['WebGL', (function(){ try { return !!document.createElement('canvas').getContext('webgl'); } catch(e){ return false; }})()],
  ['WebGL2', (function(){ try { return !!document.createElement('canvas').getContext('webgl2'); } catch(e){ return false; }})()],
  ['Codec: video/mp4', !!document.createElement('video').canPlayType('video/mp4; codecs="avc1.42E01E"')],
  ['Codec: video/webm VP8', !!document.createElement('video').canPlayType('video/webm; codecs="vp8"')],
  ['Codec: video/webm VP9', !!document.createElement('video').canPlayType('video/webm; codecs="vp9"')],
];
checks.forEach(function(c) {
  var span = document.createElement('div');
  span.innerHTML = (c[1] ? '<span class="ok">✓</span>' : '<span class="fail">✗</span>') + ' ' + c[0];
  caps.appendChild(span);
});

// 2. Video playback
var v = document.getElementById('v');
var vs = document.getElementById('vstatus');
v.oncanplay = function() { vs.innerHTML = '<span class="ok">Video can play</span>'; };
v.onplaying = function() { vs.innerHTML = '<span class="ok">Video playing!</span>'; };
v.onerror = function() { vs.innerHTML = '<span class="fail">Video error: ' + (v.error ? v.error.message : 'unknown') + '</span>'; };

// 3. Camera/Mic
function testCamera() {
  var cs = document.getElementById('camstatus');
  cs.innerHTML = 'Requesting camera...';
  L('Requesting getUserMedia({video:true})...');
  navigator.mediaDevices.getUserMedia({video: true})
    .then(function(stream) {
      document.getElementById('cam').srcObject = stream;
      cs.innerHTML = '<span class="ok">Camera active!</span>';
      L('Camera stream obtained: ' + stream.getVideoTracks().length + ' video tracks');
    })
    .catch(function(e) {
      cs.innerHTML = '<span class="fail">Camera error: ' + e.message + '</span>';
      L('Camera error: ' + e.name + ': ' + e.message);
    });
}
function testMic() {
  var cs = document.getElementById('camstatus');
  cs.innerHTML = 'Requesting microphone...';
  L('Requesting getUserMedia({audio:true})...');
  navigator.mediaDevices.getUserMedia({audio: true})
    .then(function(stream) {
      cs.innerHTML = '<span class="ok">Microphone active! Tracks: ' + stream.getAudioTracks().length + '</span>';
      L('Mic stream obtained: ' + stream.getAudioTracks().length + ' audio tracks');
    })
    .catch(function(e) {
      cs.innerHTML = '<span class="fail">Mic error: ' + e.message + '</span>';
      L('Mic error: ' + e.name + ': ' + e.message);
    });
}

// 4. RTCPeerConnection
function testPeerConnection() {
  var ps = document.getElementById('pcstatus');
  ps.innerHTML = 'Testing...';
  L('Creating RTCPeerConnection...');
  try {
    var pc = new RTCPeerConnection({iceServers: [{urls: 'stun:stun.l.google.com:19302'}]});
    pc.onicecandidate = function(e) {
      if (e.candidate) {
        L('ICE candidate: ' + e.candidate.candidate.substr(0, 80) + '...');
      }
    };
    pc.createDataChannel('test');
    pc.createOffer().then(function(offer) {
      L('SDP offer created (' + offer.sdp.length + ' bytes)');
      return pc.setLocalDescription(offer);
    }).then(function() {
      ps.innerHTML = '<span class="ok">RTCPeerConnection works! SDP offer created.</span>';
      L('setLocalDescription succeeded. WebRTC is functional.');
    }).catch(function(e) {
      ps.innerHTML = '<span class="fail">PeerConnection error: ' + e.message + '</span>';
      L('PeerConnection error: ' + e.message);
    });
  } catch(e) {
    ps.innerHTML = '<span class="fail">RTCPeerConnection not available: ' + e.message + '</span>';
    L('RTCPeerConnection error: ' + e.message);
  }
}
</script>
</body>
</html>"#;

    std::fs::write(&path, html).expect("Failed to write WebRTC test HTML");
    let canon = path.to_string_lossy().replace('\\', "/");
    format!("file:///{}", canon.trim_start_matches('/'))
}

/// Write a video test page to disk and return its `file://` URL.
pub fn write_video_test_page() -> String {
    let dir = crate::app_data_dir().join("cef_cache");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join("video_test.html");

    let html = r#"<!DOCTYPE html>
<html>
<head>
<style>
  body { margin: 0; background: #000; overflow: hidden; }
  video { width: 100vw; height: 100vh; object-fit: contain; display: block; }
  #status { position: fixed; top: 10px; left: 10px; color: lime;
            font: 16px monospace; z-index: 10; }
</style>
</head>
<body>
<div id="status">Loading video...</div>
<video id="v" autoplay muted playsinline loop>
  <source src="https://www.w3schools.com/html/mov_bbb.mp4" type="video/mp4">
  <source src="https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.webm" type="video/webm">
</video>
<script>
var s = document.getElementById('status');
var v = document.getElementById('v');

v.onloadstart  = function() { s.textContent = 'Video loading...'; };
v.oncanplay    = function() { s.textContent = 'Can play, starting...';
                               v.play().catch(function(e) {
                                 s.textContent = 'Play error: ' + e.message;
                               }); };
v.onplaying    = function() { s.textContent = 'Playing!';
                               setTimeout(function() { s.style.display = 'none'; }, 2000); };
v.onwaiting    = function() { s.textContent = 'Buffering...'; };
v.onstalled    = function() { s.textContent = 'Stalled...'; };
v.onerror      = function() { s.textContent = 'Error: ' +
                               (v.error ? v.error.message : 'unknown'); };

// Fallback: retry play after 2s if still paused.
setTimeout(function() {
  if (v.paused) {
    s.textContent = 'Retrying play()...';
    v.play().catch(function(e) {
      s.textContent = 'Retry failed: ' + e.message;
    });
  }
}, 2000);
</script>
</body>
</html>"#;

    std::fs::write(&path, html).expect("Failed to write video test HTML");
    let canon = path.to_string_lossy().replace('\\', "/");
    format!("file:///{}", canon.trim_start_matches('/'))
}

/// Initialize the overlay backend. Must be called on the main UI thread.
pub fn init(url: &str) -> Result<(), wew::Error> {
    let backend = OverlayBackend::new(url)?;
    *BACKEND.lock().unwrap() = Some(backend);
    Ok(())
}

/// Drive the overlay backend. Must be called on the main UI thread every event cycle.
pub fn poll() {
    if let Ok(mut guard) = BACKEND.lock() {
        if let Some(backend) = guard.as_mut() {
            backend.poll_internal();
        }
    }
}

// ---------------------------------------------------------------------------
// CEF initialization flag
// ---------------------------------------------------------------------------

static CEF_OVERLAY_INITIALIZED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Runtime observer
// ---------------------------------------------------------------------------

struct OverlayRuntimeObserver;

impl RuntimeHandler for OverlayRuntimeObserver {
    fn on_context_initialized(&self) {
        CEF_OVERLAY_INITIALIZED.store(true, Ordering::Release);
        SignalToUI::set_ui_signal();
    }
}

impl MessagePumpRuntimeHandler for OverlayRuntimeObserver {
    fn on_schedule_message_pump_work(&self, _delay: u64) {
        SignalToUI::set_ui_signal();
    }
}

// ---------------------------------------------------------------------------
// WebView observer (native window mode — much simpler than windowless)
// ---------------------------------------------------------------------------

struct OverlayWebViewObserver;

impl WebViewHandler for OverlayWebViewObserver {
    fn on_state_change(&self, state: WebViewState) {
        makepad_widgets::log!("Overlay: WebView state -> {:?}", state);
    }

    fn on_title_change(&self, title: &str) {
        makepad_widgets::log!("Overlay: title -> {}", title);
    }
}

// ---------------------------------------------------------------------------
// OverlayBackend
// ---------------------------------------------------------------------------

struct OverlayBackend {
    message_loop: MessagePumpLoop,
    runtime: Runtime<MessagePumpLoop, NativeWindowWebView>,
    webview: Option<WebView<NativeWindowWebView>>,
    cef_hwnd: Option<isize>,
    parent_hwnd: Option<isize>,
    dpi_factor: f64,
    url: String,
    last_bounds: (i32, i32, u32, u32),
    currently_visible: bool,
    last_draw_counter: u64,
    last_draw_time: Instant,
}

impl OverlayBackend {
    fn new(url: &str) -> Result<Self, wew::Error> {
        let helper_path = Self::helper_exe_path();
        let cache_path = Self::cache_dir();

        let message_loop = MessagePumpLoop::default();
        let runtime = message_loop
            .create_runtime_attributes_builder::<NativeWindowWebView>()
            .with_browser_subprocess_path(&helper_path)
            .with_root_cache_path(&cache_path)
            .with_cache_path(&cache_path)
            .with_log_severity(LogLevel::Info)
            .build()
            .create_runtime(OverlayRuntimeObserver)?;

        Ok(Self {
            message_loop,
            runtime,
            webview: None,
            cef_hwnd: None,
            parent_hwnd: None,
            dpi_factor: 1.0,
            url: url.to_string(),
            last_bounds: (0, 0, 0, 0),
            // Start hidden; show only when the widget's draw_walk fires.
            currently_visible: false,
            last_draw_counter: 0,
            last_draw_time: Instant::now(),
        })
    }

    fn poll_internal(&mut self) {
        self.message_loop.poll();

        // Find parent HWND lazily (it may not exist yet during early startup).
        if self.parent_hwnd.is_none() {
            if let Some(hwnd) = platform_windows::find_makepad_hwnd() {
                self.parent_hwnd = Some(hwnd);
                self.dpi_factor = platform_windows::get_dpi_factor(hwnd);
                makepad_widgets::log!(
                    "Overlay: Found Makepad HWND: 0x{:X}, DPI factor: {}",
                    hwnd,
                    self.dpi_factor
                );
            }
        }

        // Create browser once CEF is initialized and parent HWND is found.
        if self.webview.is_none()
            && CEF_OVERLAY_INITIALIZED.load(Ordering::Acquire)
            && self.parent_hwnd.is_some()
        {
            self.create_browser();
        }

        // Retry HWND retrieval — window_handle() may return None right
        // after create_webview because the native window is created async.
        if self.cef_hwnd.is_none() {
            if let Some(webview) = &self.webview {
                if let Some(wew::raw_window_handle::RawWindowHandle::Win32(handle)) =
                    webview.window_handle()
                {
                    let cef_hwnd = handle.hwnd.get();
                    self.cef_hwnd = Some(cef_hwnd);
                    makepad_widgets::log!("Overlay: CEF child HWND obtained: 0x{:X}", cef_hwnd);
                    // Force repositioning on next check by invalidating last_bounds.
                    self.last_bounds = (0, 0, 0, 0);
                }
            }
        }

        // Reposition child window to match widget rect.
        if let Some(cef_hwnd) = self.cef_hwnd {
            let x = OVERLAY_X.load(Ordering::Relaxed);
            let y = OVERLAY_Y.load(Ordering::Relaxed);
            let w = OVERLAY_W.load(Ordering::Relaxed);
            let h = OVERLAY_H.load(Ordering::Relaxed);
            let bounds = (x, y, w, h);
            if w > 0 && h > 0 && bounds != self.last_bounds {
                let dpi = self.dpi_factor;
                platform_windows::reposition_child(
                    cef_hwnd,
                    (x as f64 * dpi) as i32,
                    (y as f64 * dpi) as i32,
                    (w as f64 * dpi) as i32,
                    (h as f64 * dpi) as i32,
                );
                self.last_bounds = bounds;
            }

            // TODO: Add show/hide logic to hide overlay when the welcome
            // screen is not the active tab. For now, always visible.
        }
    }

    fn create_browser(&mut self) {
        let parent_hwnd = self.parent_hwnd.unwrap();

        // Build a Win32 window handle pointing at the Makepad parent.
        let parent_handle = wew::raw_window_handle::Win32WindowHandle::new(
            std::num::NonZeroIsize::new(parent_hwnd).unwrap(),
        );
        let raw_handle = wew::raw_window_handle::RawWindowHandle::Win32(parent_handle);

        let attrs = WebViewAttributesBuilder::default()
            .with_window_handle(raw_handle)
            .build();

        match self
            .runtime
            .create_webview(&self.url, attrs, OverlayWebViewObserver)
        {
            Ok(webview) => {
                // Retrieve the CEF child window handle for positioning.
                if let Some(wew::raw_window_handle::RawWindowHandle::Win32(handle)) =
                    webview.window_handle()
                {
                    let cef_hwnd = handle.hwnd.get();
                    self.cef_hwnd = Some(cef_hwnd);
                    makepad_widgets::log!("Overlay: CEF child HWND: 0x{:X}", cef_hwnd);

                    // Apply initial positioning from whatever the widget has reported.
                    let x = OVERLAY_X.load(Ordering::Relaxed);
                    let y = OVERLAY_Y.load(Ordering::Relaxed);
                    let w = OVERLAY_W.load(Ordering::Relaxed);
                    let h = OVERLAY_H.load(Ordering::Relaxed);
                    if w > 0 && h > 0 {
                        let dpi = self.dpi_factor;
                        platform_windows::reposition_child(
                            cef_hwnd,
                            (x as f64 * dpi) as i32,
                            (y as f64 * dpi) as i32,
                            (w as f64 * dpi) as i32,
                            (h as f64 * dpi) as i32,
                        );
                        self.last_bounds = (x, y, w, h);
                    }
                }

                makepad_widgets::log!("Overlay: Browser created for URL: {}", self.url);
                self.webview = Some(webview);
            }
            Err(e) => {
                makepad_widgets::error!("Overlay: Failed to create browser: {:?}", e);
            }
        }
    }

    fn helper_exe_path() -> String {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();
        let name = if cfg!(target_os = "windows") {
            "robrix-cef-helper.exe"
        } else {
            "robrix-cef-helper"
        };
        exe_dir.join(name).to_string_lossy().into_owned()
    }

    fn cache_dir() -> String {
        let dir = crate::app_data_dir().join("cef_cache");
        std::fs::create_dir_all(&dir).ok();
        dir.to_string_lossy().into_owned()
    }
}

// ---------------------------------------------------------------------------
// WebViewBackend trait implementation
// ---------------------------------------------------------------------------

impl super::backend::WebViewBackend for OverlayBackend {
    fn load_url(&mut self, url: &str) {
        self.url = url.to_string();
        // TODO: if browser already exists, navigate to new URL
    }

    fn set_bounds(&mut self, x: i32, y: i32, width: u32, height: u32) {
        if let Some(cef_hwnd) = self.cef_hwnd {
            platform_windows::reposition_child(cef_hwnd, x, y, width as i32, height as i32);
        }
    }

    fn set_visible(&mut self, visible: bool) {
        if let Some(cef_hwnd) = self.cef_hwnd {
            platform_windows::show_child(cef_hwnd, visible);
        }
    }

    fn send_message(&self, message: &str) {
        if let Some(webview) = &self.webview {
            webview.send_message(message);
        }
    }

    fn poll(&mut self) {
        self.poll_internal();
    }
}
