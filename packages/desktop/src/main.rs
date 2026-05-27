use dioxus::prelude::*;

use ui::{Navbar, SidebarLink};
use views::{Home, SysInfoView, ConfigView, InjectionView, AnalysisView, LiveView, FileTransferView};

mod views;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(DesktopNavbar)]
    #[route("/")]
    Home {},
    #[route("/sysinfo")]
    SysInfoView {},
    #[route("/analysis")]
    AnalysisView {},
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
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cites-desktop.log")
        .expect("failed to open /tmp/cites-desktop.log");
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::sync::Mutex::new(log_file))
        .init();
    dioxus::LaunchBuilder::new()
        .with_cfg(dioxus::desktop::Config::new().with_window(
            dioxus::desktop::WindowBuilder::new().with_title("CITES"),
        ))
        .launch(App);
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

/// A desktop-specific Router around the shared `Navbar` component.
/// Allows us to use the desktop-specific `Route` enum.
#[component]
fn DesktopNavbar() -> Element {
    use_context_provider(|| platform::ConnectionService::new());
    ui::setup_loop_coroutine();
    ui::load_persisted_settings();

    rsx! {
        Navbar {
            SidebarLink { to: Route::Home {}, "Connection Status" }
            SidebarLink { to: Route::SysInfoView {}, "SysInfo" }
            SidebarLink { to: Route::AnalysisView {}, "Analysis" }
            SidebarLink { to: Route::ConfigView {}, "Configuration" }
            SidebarLink { to: Route::InjectionView {}, "Injection" }
            SidebarLink { to: Route::LiveView {}, "Live" }
            SidebarLink { to: Route::FileTransferView {}, "File Transfer" }
        }

        Outlet::<Route> {}
    }
}
