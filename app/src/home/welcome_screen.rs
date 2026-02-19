use makepad_widgets::*;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    use crate::shared::web_view_widget::WebViewWidget;

    pub WelcomeScreen = <View> {
        width: Fill, height: Fill

        // PoC: CEF-rendered web content filling the welcome panel.
        web_view = <WebViewWidget> {
            width: Fill, height: Fill,
        }
    }
}
