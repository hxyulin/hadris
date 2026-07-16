#[path = "../app.rs"]
mod app;

fn main() -> anyhow::Result<()> {
    app::run()
}
