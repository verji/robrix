use std::path::PathBuf;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, Ordering},
};
use std::time::Instant;

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

/// Whether the browser page has finished loading (ready to receive messages).
static PAGE_LOADED: AtomicBool = AtomicBool::new(false);

/// Video download state: 0=idle, 1=downloading, 2=complete, 3=failed.
static VIDEO_DOWNLOAD_STATE: AtomicU8 = AtomicU8::new(0);
/// Video download progress: 0–100 percent.
static VIDEO_DOWNLOAD_PROGRESS: AtomicU8 = AtomicU8::new(0);
/// Path to the downloaded video file (set when download completes).
static VIDEO_LOCAL_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
/// The localhost port of the file-serving HTTP server (set once started).
static FILE_SERVER_PORT: AtomicU32 = AtomicU32::new(0);

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

/// Returns `(state, progress)` where state is 0=idle, 1=downloading, 2=complete, 3=failed.
pub fn video_download_status() -> (u8, u8) {
    (
        VIDEO_DOWNLOAD_STATE.load(Ordering::Relaxed),
        VIDEO_DOWNLOAD_PROGRESS.load(Ordering::Relaxed),
    )
}

/// Downloads a video from `url` to the local CEF cache directory.
///
/// If a file already exists with a size >= `expected_size_hint`, the download is skipped
/// (cache hit). Progress is reported via atomics and `SignalToUI`.
pub async fn download_video(url: &str, expected_size_hint: Option<u64>) -> Result<PathBuf, String> {
    use futures_util::StreamExt;

    makepad_widgets::log!("download_video: starting for {}", url);

    let filename = url.rsplit('/').next().unwrap_or("video.mp4");
    let dir = crate::app_data_dir().join("cef_shader_poc").join("videos");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir failed: {e}"))?;
    let path = dir.join(filename);

    // Cache hit: file exists with sufficient size.
    if let Ok(meta) = std::fs::metadata(&path) {
        if let Some(expected) = expected_size_hint {
            if meta.len() >= expected {
                makepad_widgets::log!("Video cache hit: {:?} ({} bytes)", path, meta.len());
                *VIDEO_LOCAL_PATH.lock().unwrap() = Some(path.clone());
                VIDEO_DOWNLOAD_PROGRESS.store(100, Ordering::Relaxed);
                VIDEO_DOWNLOAD_STATE.store(2, Ordering::Release);
                SignalToUI::set_ui_signal();
                return Ok(path);
            }
            makepad_widgets::log!("download_video: partial file {} bytes < expected {}, re-downloading", meta.len(), expected);
        }
    }

    VIDEO_DOWNLOAD_STATE.store(1, Ordering::Release);
    VIDEO_DOWNLOAD_PROGRESS.store(0, Ordering::Relaxed);
    SignalToUI::set_ui_signal();

    makepad_widgets::log!("download_video: sending HTTP request...");
    let response = reqwest::get(url).await.map_err(|e| {
        VIDEO_DOWNLOAD_STATE.store(3, Ordering::Release);
        SignalToUI::set_ui_signal();
        format!("HTTP request failed: {e}")
    })?;
    makepad_widgets::log!("download_video: got response, status={}, content_length={:?}",
        response.status(), response.content_length());

    let total = response.content_length().or(expected_size_hint).unwrap_or(0);
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(&path).map_err(|e| {
        VIDEO_DOWNLOAD_STATE.store(3, Ordering::Release);
        SignalToUI::set_ui_signal();
        format!("File create failed: {e}")
    })?;

    let mut downloaded: u64 = 0;
    let mut last_pct: u8 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            VIDEO_DOWNLOAD_STATE.store(3, Ordering::Release);
            SignalToUI::set_ui_signal();
            format!("Download stream error: {e}")
        })?;

        std::io::Write::write_all(&mut file, &chunk).map_err(|e| {
            VIDEO_DOWNLOAD_STATE.store(3, Ordering::Release);
            SignalToUI::set_ui_signal();
            format!("File write failed: {e}")
        })?;

        downloaded += chunk.len() as u64;
        let pct = if total > 0 {
            ((downloaded * 100) / total).min(100) as u8
        } else {
            0
        };

        if pct != last_pct {
            last_pct = pct;
            VIDEO_DOWNLOAD_PROGRESS.store(pct, Ordering::Relaxed);
            SignalToUI::set_ui_signal();
        }
    }

    *VIDEO_LOCAL_PATH.lock().unwrap() = Some(path.clone());
    VIDEO_DOWNLOAD_PROGRESS.store(100, Ordering::Relaxed);
    VIDEO_DOWNLOAD_STATE.store(2, Ordering::Release);
    SignalToUI::set_ui_signal();

    makepad_widgets::log!("Video download complete: {:?} ({} bytes)", path, downloaded);
    Ok(path)
}

/// Starts a minimal HTTP file server on localhost that serves a single video file
/// with support for `Range` requests (HTTP 206 Partial Content).
///
/// This is needed because CEF/Chromium's media player requires range requests to
/// seek through large video files, and `file://` URLs don't support that.
///
/// Stores the port in `FILE_SERVER_PORT` and runs forever on the tokio runtime.
pub async fn start_file_server(video_path: PathBuf) -> Result<u16, String> {
    use std::io::{Read, Seek, SeekFrom};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await
        .map_err(|e| format!("bind failed: {e}"))?;
    let port = listener.local_addr()
        .map_err(|e| format!("local_addr failed: {e}"))?.port();

    FILE_SERVER_PORT.store(port as u32, Ordering::Release);
    SignalToUI::set_ui_signal();
    makepad_widgets::log!("File server started on http://127.0.0.1:{}", port);

    let file_size = std::fs::metadata(&video_path)
        .map_err(|e| format!("metadata failed: {e}"))?.len();

    // Spawn the accept loop — runs until the app exits.
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, peer)) = listener.accept().await else { continue };
            let path = video_path.clone();
            let size = file_size;

            tokio::spawn(async move {
                makepad_widgets::log!("File server: new connection from {}", peer);

                // HTTP/1.1 keep-alive loop: handle multiple requests on the
                // same TCP connection. Chromium's media player often reuses a
                // connection for follow-up Range requests.
                loop {
                    // Read the HTTP request. Use a generous timeout so idle
                    // connections are cleaned up.
                    let mut buf = vec![0u8; 4096];
                    let n = match tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        tokio::io::AsyncReadExt::read(&mut stream, &mut buf),
                    ).await {
                        Ok(Ok(0)) | Err(_) => {
                            makepad_widgets::log!("File server: connection closed/timeout");
                            return;
                        }
                        Ok(Ok(n)) => n,
                        Ok(Err(_)) => return,
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let req_line = request.lines().next().unwrap_or("?");
                    let is_head = req_line.starts_with("HEAD ");

                    // Parse Range header: "Range: bytes=START-END" or "Range: bytes=START-"
                    let range_line = request.lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("range:"));
                    let has_range = range_line.is_some();

                    let (start, mut end) = if let Some(rl) = range_line {
                        parse_range(rl, size)
                    } else {
                        (0, size - 1)
                    };

                    // Cap responses so each is complete and small enough for
                    // Chromium to consume fully. Chromium sees the total size
                    // in Content-Range and chains follow-up Range requests.
                    const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024; // 4MB
                    if (end - start + 1) > MAX_RESPONSE_BYTES {
                        end = start + MAX_RESPONSE_BYTES - 1;
                    }
                    let length = end - start + 1;

                    // Use 206 whenever we serve less than the full file.
                    let use_partial = has_range || length < size;
                    let header = if use_partial {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\n\
                             Content-Type: video/mp4\r\n\
                             Content-Length: {length}\r\n\
                             Content-Range: bytes {start}-{end}/{size}\r\n\
                             Accept-Ranges: bytes\r\n\
                             \r\n"
                        )
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\n\
                             Content-Type: video/mp4\r\n\
                             Content-Length: {size}\r\n\
                             Accept-Ranges: bytes\r\n\
                             \r\n"
                        )
                    };

                    let status_code = if use_partial { 206 } else { 200 };
                    makepad_widgets::log!(
                        "File server: {} -> {} (bytes {}-{}/{}) range_hdr={}",
                        req_line, status_code, start, end, size, has_range
                    );

                    if stream.write_all(header.as_bytes()).await.is_err() { return; }

                    if is_head { continue; } // keep connection open for next request

                    // Stream the file in 64KB chunks.
                    let Ok(mut file) = std::fs::File::open(&path) else { return };
                    if file.seek(SeekFrom::Start(start)).is_err() { return; }

                    let mut sent: u64 = 0;
                    let mut chunk = vec![0u8; 65536];
                    let mut remaining = length;
                    let mut write_ok = true;
                    while remaining > 0 {
                        let to_read = (remaining as usize).min(chunk.len());
                        let Ok(n) = file.read(&mut chunk[..to_read]) else {
                            makepad_widgets::log!("File server: read error after {} bytes", sent);
                            break;
                        };
                        if n == 0 { break; }
                        if stream.write_all(&chunk[..n]).await.is_err() {
                            makepad_widgets::log!(
                                "File server: write error after {}/{} bytes (client closed?)",
                                sent, length
                            );
                            write_ok = false;
                            break;
                        }
                        sent += n as u64;
                        remaining -= n as u64;
                    }
                    makepad_widgets::log!("File server: done streaming {}/{} bytes", sent, length);

                    // If the write failed, the connection is broken — stop the loop.
                    if !write_ok { return; }

                    // Otherwise, loop back to read the next request on this connection.
                }
            });
        }
    });

    Ok(port)
}

/// Parse an HTTP Range header like "Range: bytes=0-1023" into (start, end).
fn parse_range(header: &str, file_size: u64) -> (u64, u64) {
    let default = (0, file_size - 1);
    let Some(eq) = header.find('=') else { return default };
    let range_spec = header[eq + 1..].trim();
    let Some(dash) = range_spec.find('-') else { return default };

    let start: u64 = range_spec[..dash].parse().unwrap_or(0);
    let end: u64 = if dash + 1 < range_spec.len() {
        range_spec[dash + 1..].trim().parse().unwrap_or(file_size - 1)
    } else {
        file_size - 1
    };

    (start.min(file_size - 1), end.min(file_size - 1))
}

/// Manages the CEF runtime lifecycle: initialization, browser creation, and polling.
pub struct CefManager {
    message_loop: MessagePumpLoop,
    runtime: Runtime<MessagePumpLoop, WindowlessRenderWebView>,
    webview: Option<WebView<WindowlessRenderWebView>>,
    browser_url: String,
    browser_width: u32,
    browser_height: u32,
    /// Whether we've already sent the video URL to the browser.
    video_url_sent: bool,
    /// The last progress percentage we sent to the browser, to avoid duplicates.
    last_sent_progress: u8,
    /// When this manager was created, used for the initialization timeout fallback.
    created_at: Instant,
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
            video_url_sent: false,
            last_sent_progress: 0,
            created_at: Instant::now(),
        })
    }

    /// Drive CEF's message loop. Must be called on the main thread, ideally
    /// on every Makepad event cycle.
    pub fn poll(&mut self) {
        self.message_loop.poll();

        // Once CEF is initialized and we haven't created a browser yet, do so.
        // Fallback: if on_context_initialized() hasn't fired within 3 seconds
        // (e.g., because CEF connected to an existing browser session), force it.
        if self.webview.is_none() && !CEF_INITIALIZED.load(Ordering::Acquire) {
            static LOGGED_WAITING: AtomicBool = AtomicBool::new(false);
            if !LOGGED_WAITING.swap(true, Ordering::Relaxed) {
                makepad_widgets::log!("CEF: Waiting for on_context_initialized()...");
            }
            if self.created_at.elapsed().as_secs() >= 3 {
                makepad_widgets::log!("CEF: on_context_initialized() timed out after 3s — \
                    forcing initialization (likely reusing existing browser session).");
                CEF_INITIALIZED.store(true, Ordering::Release);
            }
        }
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

        // Dispatch any pending click requested via `request_click()`.
        let cx = PENDING_CLICK_X.load(Ordering::Relaxed);
        let cy = PENDING_CLICK_Y.load(Ordering::Relaxed);
        if cx != i32::MIN && cy != i32::MIN {
            self.send_click(cx, cy);
            PENDING_CLICK_X.store(i32::MIN, Ordering::Relaxed);
            PENDING_CLICK_Y.store(i32::MIN, Ordering::Relaxed);
        }

        // Send video download status to the browser via MessageTransport.
        // Wait until the page has loaded so the JS MessageTransport.on() handler is registered.
        if let Some(webview) = &self.webview {
            if !self.video_url_sent && PAGE_LOADED.load(Ordering::Acquire) {
                let state = VIDEO_DOWNLOAD_STATE.load(Ordering::Acquire);
                match state {
                    1 => {
                        // Downloading — send progress updates.
                        let pct = VIDEO_DOWNLOAD_PROGRESS.load(Ordering::Relaxed);
                        if pct != self.last_sent_progress {
                            self.last_sent_progress = pct;
                            let msg = format!(r#"{{"type":"progress","pct":{}}}"#, pct);
                            webview.send_message(&msg);
                        }
                    }
                    2 => {
                        // Complete — wait for the file server to be ready, then
                        // write a playback HTML page and navigate to it.
                        // HTML5 <video> actively manages range request loading,
                        // unlike direct URL navigation which may not chain requests.
                        let port = FILE_SERVER_PORT.load(Ordering::Acquire);
                        if port > 0 {
                            if let Some(path) = VIDEO_LOCAL_PATH.lock().unwrap().as_ref() {
                                let filename = path.file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "video.mp4".to_string());
                                let video_url = format!("http://127.0.0.1:{}/{}", port, filename);
                                let play_page = Self::write_video_play_page(&video_url);
                                let msg = format!(
                                    r#"{{"type":"navigate","url":"{}"}}"#,
                                    play_page,
                                );
                                webview.send_message(&msg);
                                self.video_url_sent = true;
                                makepad_widgets::log!(
                                    "CEF: Navigate to play page: {} (video: {})",
                                    play_page, video_url
                                );
                            }
                        }
                    }
                    3 => {
                        // Failed — send error.
                        let msg = r#"{"type":"error","message":"Video download failed"}"#;
                        webview.send_message(msg);
                        self.video_url_sent = true;
                        makepad_widgets::error!("CEF: Video download failed, notified browser");
                    }
                    _ => {} // idle (0) — nothing to do yet.
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
    ///
    /// Uses a unique subdirectory (`cef_shader_poc`) to isolate this exploration
    /// from other CEF-based branches (e.g., `taj/minimal-widget-poc` uses `cef_cache`).
    /// This prevents cross-instance interference with lock files and session detection.
    ///
    /// Also removes stale CEF lock files to handle restarts of the same instance.
    fn cache_dir() -> String {
        let dir = crate::app_data_dir().join("cef_shader_poc");
        std::fs::create_dir_all(&dir).ok();

        // Remove stale CEF singleton lock files that prevent clean initialization
        // after an unclean shutdown of a previous run of THIS same exploration.
        for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
            let lock_file = dir.join(name);
            if lock_file.exists() {
                makepad_widgets::log!("CEF: Removing stale lock file: {:?}", lock_file);
                std::fs::remove_file(&lock_file).ok();
            }
        }

        dir.to_string_lossy().into_owned()
    }

    /// Write a test HTML page to disk and return its `file://` URL.
    ///
    /// The page starts with a loading/progress UI. Once Rust finishes downloading
    /// the video, it sends a JSON message via `WebView::send_message()` containing
    /// the local `file://` URL, and the page switches to video playback.
    pub fn write_video_test_page() -> String {
        let dir = crate::app_data_dir().join("cef_shader_poc");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("video_test.html");

        let html = r#"<!DOCTYPE html>
<html>
<head>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { background: #111; color: #eee; font-family: system-ui, sans-serif;
         display: flex; align-items: center; justify-content: center;
         width: 100vw; height: 100vh; overflow: hidden; }

  /* Loading overlay */
  #loading { text-align: center; }
  #loading h2 { margin-bottom: 20px; font-weight: 300; }
  #bar-bg { width: 320px; height: 12px; background: #333; border-radius: 6px;
            overflow: hidden; margin: 0 auto 12px; }
  #bar-fg { width: 0%; height: 100%; background: #4caf50; border-radius: 6px;
            transition: width 0.3s ease; }
  #pct-text { font-size: 14px; color: #aaa; }
  #error-text { color: #f44; font-size: 14px; margin-top: 10px; display: none; }

  /* Video (hidden until ready) */
  video { width: 100vw; height: 100vh; object-fit: contain; display: none; }
  #vid-status { position: fixed; top: 10px; left: 10px; color: lime;
                font: 14px monospace; z-index: 10; display: none; }
</style>
</head>
<body>

<div id="loading">
  <h2>Downloading video…</h2>
  <div id="bar-bg"><div id="bar-fg"></div></div>
  <div id="pct-text">0%</div>
  <div id="error-text"></div>
</div>

<div id="vid-status"></div>
<video id="v" autoplay muted playsinline loop></video>

<script>
var loading  = document.getElementById('loading');
var barFg    = document.getElementById('bar-fg');
var pctText  = document.getElementById('pct-text');
var errorEl  = document.getElementById('error-text');
var vidStat  = document.getElementById('vid-status');
var v        = document.getElementById('v');

// Video event handlers (active once src is set).
v.oncanplay  = function() { vidStat.textContent = 'Buffered, starting…';
                             vidStat.style.display = 'block';
                             v.play().catch(function(e) {
                               vidStat.textContent = 'Play error: ' + e.message;
                             }); };
v.onplaying  = function() { vidStat.textContent = 'Playing';
                             setTimeout(function() { vidStat.style.display = 'none'; }, 2000); };
v.onwaiting  = function() { vidStat.textContent = 'Buffering…';
                             vidStat.style.display = 'block'; };
v.onerror    = function() { vidStat.textContent = 'Video error: ' +
                             (v.error ? v.error.message : 'unknown');
                             vidStat.style.display = 'block'; };

// Listen for messages from Rust via wew's MessageTransport bridge.
if (window.MessageTransport) {
  window.MessageTransport.on(function(raw) {
    try {
      var msg = JSON.parse(raw);
    } catch(e) { return; }

    if (msg.type === 'progress') {
      barFg.style.width = msg.pct + '%';
      pctText.textContent = msg.pct + '%';
    }
    else if (msg.type === 'navigate') {
      window.location.href = msg.url;
    }
    else if (msg.type === 'error') {
      errorEl.textContent = msg.message;
      errorEl.style.display = 'block';
      pctText.textContent = 'Download failed';
    }
  });
} else {
  pctText.textContent = 'MessageTransport not available';
}
</script>
</body>
</html>"#;

        std::fs::write(&path, html).expect("Failed to write video test HTML");

        // Convert to a file:// URL (forward slashes, no UNC prefix).
        let canon = path.to_string_lossy().replace('\\', "/");
        format!("file:///{}", canon.trim_start_matches('/'))
    }

    /// Write a playback page with the video `<source>` URL baked into the HTML.
    /// Returns the `file://` URL of the written page.
    fn write_video_play_page(video_url: &str) -> String {
        let dir = crate::app_data_dir().join("cef_shader_poc");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("video_play.html");

        let html = format!(r#"<!DOCTYPE html>
<html>
<head>
<style>
  body {{ margin: 0; background: #000; overflow: hidden; }}
  video {{ width: 100vw; height: 100vh; object-fit: contain; display: block; }}
  #status {{ position: fixed; top: 10px; left: 10px; color: lime;
            font: 14px monospace; z-index: 10; }}
</style>
</head>
<body>
<div id="status">Loading video...</div>
<video id="v" autoplay muted playsinline loop preload="auto">
  <source src="{video_url}" type="video/mp4">
</video>
<script>
var s = document.getElementById('status');
var v = document.getElementById('v');
v.oncanplay  = function() {{ s.textContent = 'Can play, starting...';
                             v.play().catch(function(e) {{
                               s.textContent = 'Play error: ' + e.message;
                             }}); }};
v.onplaying  = function() {{ s.textContent = 'Playing!';
                             setTimeout(function() {{ s.style.display = 'none'; }}, 2000); }};
v.onwaiting  = function() {{ s.textContent = 'Buffering...'; s.style.display = 'block'; }};
v.onerror    = function() {{ s.textContent = 'Error: ' +
                             (v.error ? v.error.message : 'unknown'); }};
setTimeout(function() {{
  if (v.paused) {{
    s.textContent = 'Retrying play()...';
    v.play().catch(function(e) {{
      s.textContent = 'Retry failed: ' + e.message;
    }});
  }}
}}, 2000);
</script>
</body>
</html>"#);

        std::fs::write(&path, html).expect("Failed to write video play HTML");

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
        makepad_widgets::log!("CEF: on_context_initialized() called — CEF runtime is ready.");
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
        if matches!(state, wew::webview::WebViewState::Loaded) {
            PAGE_LOADED.store(true, Ordering::Release);
            SignalToUI::set_ui_signal();
        }
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
