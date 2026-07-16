#[path = "../app.rs"]
mod app;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
