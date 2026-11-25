#[path = "../../shared/checksum.rs"]
mod checksum;
mod cli;
#[cfg(feature = "gui")]
mod discovery;
#[path = "../../shared/http_tls.rs"]
mod http_tls;
mod package;
mod station;
#[path = "../../shared/wake_catalog.rs"]
mod wake_catalog;
#[cfg(feature = "gui")]
mod window;

use std::{env, process::ExitCode};

fn main() -> ExitCode {
    match cli::parse(env::args_os().skip(1)) {
        Ok(cli::Action::Gui) => gui(),
        Ok(cli::Action::Help) => {
            print!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Ok(cli::Action::Version) => {
            println!("goodwin-installer {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(cli::Action::Install(options)) => match cli::install(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Не удалось установить Goodwin. {error:#}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error:#}\n\n{}", cli::USAGE);
            ExitCode::from(2)
        }
    }
}

#[cfg(not(feature = "gui"))]
fn gui() -> ExitCode {
    print!("{}", cli::USAGE);
    ExitCode::SUCCESS
}

#[cfg(feature = "gui")]
fn gui() -> ExitCode {
    #[cfg(windows)]
    detach_private_console();

    match run_gui() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Не удалось открыть установщик. {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(windows, feature = "gui"))]
fn detach_private_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};

    let mut processes = [0; 2];
    // An Explorer launch creates a console with only the installer attached.
    unsafe {
        if GetConsoleProcessList(processes.as_mut_ptr(), processes.len() as u32) == 1 {
            FreeConsole();
        }
    }
}

#[cfg(feature = "gui")]
fn run_gui() -> eframe::Result {
    use eframe::egui;

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../icons/goodwin.png"))
        .expect("the embedded Goodwin icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_inner_size([580.0, 470.0])
            .with_min_inner_size([460.0, 370.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Установщик Goodwin",
        options,
        Box::new(|creation| {
            window::configure(&creation.egui_ctx);
            Ok(Box::new(window::InstallerWindow::new(&creation.egui_ctx)))
        }),
    )
}
