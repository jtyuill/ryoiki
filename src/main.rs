mod app;
mod stages;
mod vndb;

use relm4::RelmApp;

fn main() {
    let app = RelmApp::new("dev.ryoiki.Ryoiki");
    app.run::<app::App>(());
}
