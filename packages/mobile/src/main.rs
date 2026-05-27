use dioxus::prelude::*;

use ui::{Navbar, SidebarLink};
use views::{Home, SysInfoView, ConfigView, InjectionView, LiveView, FileTransferView};

mod views;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(MobileNavbar)]
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

const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    // Main application logic

    rsx! {
        // Global app resources
        document::Link { rel: "stylesheet", href: MAIN_CSS }

        Router::<Route> {}
    }
}

/// A mobile-specific Router around the shared `Navbar` component
/// which allows us to use the mobile-specific `Route` enum.
#[component]
fn MobileNavbar() -> Element {
    use_context_provider(|| platform::ConnectionService::new());
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
