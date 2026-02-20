use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
};

use makepad_widgets::SignalToUI;
use wew::{
    MessageLoopAbstract, MessagePumpLoop, WindowlessRenderWebView,
    events::{MouseButton, MouseEvent, Position},
    runtime::{LogLevel, MessagePumpRuntimeHandler, Runtime, RuntimeHandler},
    webview::{
        Frame, FrameType, WebView, WebViewAttributesBuilder, WebViewHandler,
        WindowlessRenderWebViewHandler,
    },
};

/// Whether CEF's context has been initialized and is ready to create browsers.
static CEF_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// The latest rendered frame from CEF, shared between CEF's callback and the UI thread.
static LATEST_FRAME: Mutex<Option<FrameData>> = Mutex::new(None);

/// Desired browser size, set by the widget when it detects its layout size.
static DESIRED_WIDTH: AtomicU32 = AtomicU32::new(0);
static DESIRED_HEIGHT: AtomicU32 = AtomicU32::new(0);

/// Pending click coordinates. `i32::MIN` means no click is pending.
static PENDING_CLICK_X: AtomicI32 = AtomicI32::new(i32::MIN);
static PENDING_CLICK_Y: AtomicI32 = AtomicI32::new(i32::MIN);

/// Raw frame data rendered by CEF.
pub struct FrameData {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
}

/// Takes the latest frame data, if available. Called from the UI thread.
pub fn take_latest_frame() -> Option<FrameData> {
    LATEST_FRAME.lock().ok()?.take()
}

/// Called by the widget to request a specific browser size.
pub fn set_desired_size(width: u32, height: u32) {
    DESIRED_WIDTH.store(width, Ordering::Relaxed);
    DESIRED_HEIGHT.store(height, Ordering::Relaxed);
}

/// Request a mouse click at the given coordinates (dispatched on next poll).
pub fn request_click(x: i32, y: i32) {
    PENDING_CLICK_X.store(x, Ordering::Relaxed);
    PENDING_CLICK_Y.store(y, Ordering::Relaxed);
}

/// Manages the CEF runtime lifecycle: initialization, browser creation, and polling.
pub struct CefManager {
    message_loop: MessagePumpLoop,
    runtime: Runtime<MessagePumpLoop, WindowlessRenderWebView>,
    webview: Option<WebView<WindowlessRenderWebView>>,
    browser_url: String,
    browser_width: u32,
    browser_height: u32,
    browser_created_at: Option<std::time::Instant>,
    auto_click_fired: bool,
}

impl CefManager {
    /// Creates a new CefManager. This initializes the CEF runtime but does NOT
    /// create a browser yet — that happens automatically once CEF signals readiness.
    pub fn new(url: &str, width: u32, height: u32) -> Result<Self, wew::Error> {
        let helper_path = Self::helper_exe_path();
        let cache_path = Self::cache_dir();

        let message_loop = MessagePumpLoop::default();
        let runtime = message_loop
            .create_runtime_attributes_builder::<WindowlessRenderWebView>()
            .with_browser_subprocess_path(&helper_path)
            .with_root_cache_path(&cache_path)
            .with_cache_path(&cache_path)
            .with_log_severity(LogLevel::Info)
            .build()
            .create_runtime(RuntimeObserver)?;

        Ok(Self {
            message_loop,
            runtime,
            webview: None,
            browser_url: url.to_string(),
            browser_width: width,
            browser_height: height,
            browser_created_at: None,
            auto_click_fired: false,
        })
    }

    /// Drive CEF's message loop. Must be called on the main thread, ideally
    /// on every Makepad event cycle.
    pub fn poll(&mut self) {
        self.message_loop.poll();

        // Once CEF is initialized and we haven't created a browser yet, do so.
        if self.webview.is_none() && CEF_INITIALIZED.load(Ordering::Acquire) {
            // Use the desired size from the widget if available, otherwise fall back to initial.
            let dw = DESIRED_WIDTH.load(Ordering::Relaxed);
            let dh = DESIRED_HEIGHT.load(Ordering::Relaxed);
            if dw > 0 && dh > 0 {
                self.browser_width = dw;
                self.browser_height = dh;
            }
            self.create_browser();
            self.browser_created_at = Some(std::time::Instant::now());
        }

        // If the widget reported a new size, resize the browser.
        if let Some(webview) = &self.webview {
            let dw = DESIRED_WIDTH.load(Ordering::Relaxed);
            let dh = DESIRED_HEIGHT.load(Ordering::Relaxed);
            if dw > 0 && dh > 0 && (dw != self.browser_width || dh != self.browser_height) {
                self.browser_width = dw;
                self.browser_height = dh;
                webview.resize(dw, dh);
            }
        }

        // Dispatch any pending click requested via `request_click()`.
        let cx = PENDING_CLICK_X.load(Ordering::Relaxed);
        let cy = PENDING_CLICK_Y.load(Ordering::Relaxed);
        if cx != i32::MIN && cy != i32::MIN {
            self.send_click(cx, cy);
            PENDING_CLICK_X.store(i32::MIN, Ordering::Relaxed);
            PENDING_CLICK_Y.store(i32::MIN, Ordering::Relaxed);
        }

        // Auto-click the center of the widget after a delay to start video playback.
        if !self.auto_click_fired {
            if let Some(created_at) = self.browser_created_at {
                if created_at.elapsed() > std::time::Duration::from_secs(3) {
                    let cx = (self.browser_width / 2) as i32;
                    let cy = (self.browser_height / 2) as i32;
                    self.send_click(cx, cy);
                    self.auto_click_fired = true;
                    makepad_widgets::log!(
                        "CEF: Auto-click sent at ({}, {}) to start video", cx, cy
                    );
                }
            }
        }
    }

    /// Send a mouse click (move + down + up) at the given position.
    fn send_click(&self, x: i32, y: i32) {
        if let Some(webview) = &self.webview {
            let pos = Some(Position { x, y });
            // Move cursor to the target position first (so hover state is correct).
            webview.mouse(&MouseEvent::Move(Position { x, y }));
            // Mouse down.
            webview.mouse(&MouseEvent::Click(MouseButton::Left, true, pos));
            // Mouse up.
            webview.mouse(&MouseEvent::Click(MouseButton::Left, false, pos));
        }
    }

    fn create_browser(&mut self) {
        let attrs = WebViewAttributesBuilder::default()
            .with_width(self.browser_width)
            .with_height(self.browser_height)
            .build();

        match self.runtime.create_webview(&self.browser_url, attrs, WebViewObserver) {
            Ok(webview) => {
                makepad_widgets::log!("CEF browser created for URL: {}", self.browser_url);
                self.webview = Some(webview);
            }
            Err(e) => {
                makepad_widgets::error!("Failed to create CEF browser: {:?}", e);
            }
        }
    }

    /// Returns the path to the CEF helper executable, located next to the main binary.
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

    /// Returns the cache directory for CEF data.
    fn cache_dir() -> String {
        let dir = crate::app_data_dir().join("cef_cache");
        std::fs::create_dir_all(&dir).ok();
        dir.to_string_lossy().into_owned()
    }

    /// Write a test HTML page to disk and return its `file://` URL.
    ///
    /// Using `file://` instead of `data:` so the page can fetch external
    /// resources (video files) without origin restrictions.
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

        // Convert to a file:// URL (forward slashes, no UNC prefix).
        let canon = path.to_string_lossy().replace('\\', "/");
        format!("file:///{}", canon.trim_start_matches('/'))
    }
}

// ---------------------------------------------------------------------------
// CEF runtime lifecycle handler
// ---------------------------------------------------------------------------

struct RuntimeObserver;

impl RuntimeHandler for RuntimeObserver {
    fn on_context_initialized(&self) {
        CEF_INITIALIZED.store(true, Ordering::Release);
        SignalToUI::set_ui_signal();
    }
}

impl MessagePumpRuntimeHandler for RuntimeObserver {
    fn on_schedule_message_pump_work(&self, _delay: u64) {
        // Wake Makepad so it calls poll() promptly.
        SignalToUI::set_ui_signal();
    }
}

// ---------------------------------------------------------------------------
// Per-browser event handler (receives rendered frames)
// ---------------------------------------------------------------------------

struct WebViewObserver;

impl WebViewHandler for WebViewObserver {
    fn on_state_change(&self, state: wew::webview::WebViewState) {
        makepad_widgets::log!("CEF: WebView state changed to {:?}", state);
    }

    fn on_title_change(&self, title: &str) {
        makepad_widgets::log!("CEF: Page title changed to: {}", title);
    }
}

impl WindowlessRenderWebViewHandler for WebViewObserver {
    fn on_frame(&self, frame: &Frame) {
        if !matches!(frame.ty, FrameType::View) {
            return;
        }

        // CEF delivers BGRA8 bytes. On little-endian, reinterpreting 4 bytes as a u32
        // gives exactly the layout Makepad's VecBGRAu8_32 expects:
        //   byte 0 = B, byte 1 = G, byte 2 = R, byte 3 = A
        //   → u32 = (A << 24) | (R << 16) | (G << 8) | B
        let pixels: Vec<u32> = frame
            .buffer
            .chunks_exact(4)
            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        if let Ok(mut latest) = LATEST_FRAME.lock() {
            *latest = Some(FrameData {
                pixels,
                width: frame.width,
                height: frame.height,
            });
        }

        SignalToUI::set_ui_signal();
    }
}
