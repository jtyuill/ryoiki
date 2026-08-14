mod app;
mod stages;
mod vndb;

use relm4::RelmApp;

fn main() {
    let app = RelmApp::new("dev.ryoiki.Ryoiki");
    relm4::set_global_css(".vndb-cover { min-width: 72px; min-height: 108px; }");
    app.run::<app::App>(());
}
