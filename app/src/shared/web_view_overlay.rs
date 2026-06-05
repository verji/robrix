//! Makepad widget that owns the visible area for a native CEF overlay window.
//!
//! The widget itself is just a colored placeholder. On each draw pass it
//! reports its layout rect to the overlay backend, which repositions the
//! CEF child window to cover the same area.

use makepad_widgets::*;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    pub WebViewOverlay = {{WebViewOverlay}} {
        width: Fill,
        height: Fill,
        show_bg: true,
        draw_bg: {
            fn pixel(self) -> vec4 {
                // Dark placeholder visible while CEF loads.
                return vec4(0.15, 0.15, 0.15, 1.0);
            }
        }
    }
}

#[derive(Live, LiveHook, Widget)]
pub struct WebViewOverlay {
    #[deref] view: View,
    #[rust] last_rect: (i32, i32, u32, u32),
}

impl Widget for WebViewOverlay {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let result = self.view.draw_walk(cx, scope, walk);

        // Report widget rect to the overlay backend for child window positioning.
        let rect = self.view.area().rect(cx);
        let x = rect.pos.x as i32;
        let y = rect.pos.y as i32;
        let w = rect.size.x as u32;
        let h = rect.size.y as u32;
        if w > 0 && h > 0 && (x, y, w, h) != self.last_rect {
            self.last_rect = (x, y, w, h);
            #[cfg(target_os = "windows")]
            crate::webview::overlay_backend::set_overlay_rect(x, y, w, h);
        }

        result
    }
}
