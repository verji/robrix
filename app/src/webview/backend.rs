/// Platform-agnostic interface for webview rendering backends.
///
/// Both the native overlay approach (child window) and the texture approach
/// (off-screen rendering into a Makepad shader) can implement this trait.
pub trait WebViewBackend {
    fn load_url(&mut self, url: &str);
    fn set_bounds(&mut self, x: i32, y: i32, width: u32, height: u32);
    fn set_visible(&mut self, visible: bool);
    fn send_message(&self, message: &str);
    fn poll(&mut self);
}
