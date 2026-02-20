use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

use makepad_widgets::SignalToUI;
use wew::{
    MessageLoopAbstract, MessagePumpLoop, WindowlessRenderWebView,
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

/// Manages the CEF runtime lifecycle: initialization, browser creation, and polling.
pub struct CefManager {
    message_loop: MessagePumpLoop,
    runtime: Runtime<MessagePumpLoop, WindowlessRenderWebView>,
    webview: Option<WebView<WindowlessRenderWebView>>,
    browser_url: String,
    browser_width: u32,
    browser_height: u32,
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

impl WebViewHandler for WebViewObserver {}

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
