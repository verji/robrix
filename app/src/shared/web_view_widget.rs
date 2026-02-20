use makepad_widgets::*;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    pub WebViewWidget = {{WebViewWidget}} {
        width: Fill,
        height: Fill,
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
    /// The dimensions the texture was created at.
    #[rust] tex_width: usize,
    #[rust] tex_height: usize,
    /// Last widget layout size reported to CEF.
    #[rust] last_widget_width: u32,
    #[rust] last_widget_height: u32,
}

impl Widget for WebViewWidget {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);

        // On every event cycle, check if CEF produced a new frame.
        if let Some(frame) = crate::webview::take_latest_frame() {
            self.update_texture(cx, frame);
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let result = self.view.draw_walk(cx, scope, walk);

        // After layout, report the actual pixel size to CEF.
        let rect = self.view.area().rect(cx);
        let w = rect.size.x as u32;
        let h = rect.size.y as u32;
        if w > 0 && h > 0 && (w != self.last_widget_width || h != self.last_widget_height) {
            self.last_widget_width = w;
            self.last_widget_height = h;
            crate::webview::set_desired_size(w, h);
        }

        result
    }
}

impl WebViewWidget {
    fn update_texture(&mut self, cx: &mut Cx, frame: crate::webview::FrameData) {
        let fw = frame.width as usize;
        let fh = frame.height as usize;

        // If the texture exists and the frame dimensions match, just swap pixel data.
        if self.texture.is_some() && fw == self.tex_width && fh == self.tex_height {
            let mut pixels = frame.pixels;
            self.texture.as_ref().unwrap().swap_vec_u32(cx, &mut pixels);
        } else {
            // First frame or frame dimensions changed — create a new texture.
            let texture = Texture::new_with_format(
                cx,
                TextureFormat::VecBGRAu8_32 {
                    width: fw,
                    height: fh,
                    data: Some(frame.pixels),
                    updated: TextureUpdated::Full,
                },
            );
            self.view.draw_bg.draw_vars.set_texture(0, &texture);
            self.texture = Some(texture);
            self.tex_width = fw;
            self.tex_height = fh;
        }

        self.redraw(cx);
    }
}
