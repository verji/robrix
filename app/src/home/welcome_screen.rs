use makepad_widgets::*;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    use crate::shared::web_view_overlay::WebViewOverlay;

    pub WelcomeScreen = <View> {
        width: Fill, height: Fill

        // PoC: CEF native overlay window filling the welcome panel.
        web_view = <WebViewOverlay> {
            width: Fill, height: Fill,
        }
    }
}
