#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
mod profiles;
#[cfg(windows)]
mod signing;

#[cfg(windows)]
fn main() {
    if let Err(error) = app::run() {
        app::message(&format!("Não foi possível iniciar o Assinador.\n\n{error}"));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("O Assinador para WhatsApp requer Windows.");
}
