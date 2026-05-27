use dioxus::prelude::*;

use ui::{Navbar, SidebarLink};
use views::{Home, SysInfoView, ConfigView, InjectionView, LiveView, FileTransferView};

mod views;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(WebNavbar)]
    #[route("/")]
    Home {},
    #[route("/sysinfo")]
    SysInfoView {},
    #[route("/config")]
    ConfigView {},
    #[route("/injection")]
    InjectionView {},
    #[route("/live")]
    LiveView {},
    #[route("/file-transfer")]
    FileTransferView {},
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    // Main application logic

    rsx! {
        // Global app resources
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }

        Router::<Route> {}
    }
}

/// A web-specific Router around the shared `Navbar` component
/// which allows us to use the web-specific `Route` enum.

#[component]
fn WebNavbar() -> Element {
    use_context_provider(|| platform::ConnectionService::new());
    ui::setup_loop_coroutine();
    ui::load_persisted_settings();

    rsx! {
        Navbar {
            SidebarLink { to: Route::Home {}, "Connection Status" }
            SidebarLink { to: Route::SysInfoView {}, "SysInfo" }
            SidebarLink { to: Route::ConfigView {}, "Configuration" }
            SidebarLink { to: Route::InjectionView {}, "Injection" }
            SidebarLink { to: Route::LiveView {}, "Live" }
            SidebarLink { to: Route::FileTransferView {}, "File Transfer" }
        }

        Outlet::<Route> {}
    }
}
