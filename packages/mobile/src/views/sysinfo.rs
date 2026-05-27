use dioxus::prelude::*;
use ui::SysInfo;

#[component]
pub fn SysInfoView() -> Element {
    rsx! {
        SysInfo {}
    }
}
