#[path = "../../platform/windows/build.rs"]
mod windows_resources;
fn main() {
    windows_resources::main();
}
