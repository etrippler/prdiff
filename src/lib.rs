mod app;
mod args;
mod git;
mod highlight;
mod logging;
mod model;
mod tree;
mod ui;
mod watcher;

use anyhow::Result;

pub fn run() -> Result<()> {
    // Ignore SIGPIPE to prevent crashes when writing to closed pipes
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    logging::init_logging();
    logging::init_tracing();

    let base_branch = args::parse_args()?;
    let mut app = app::App::new(base_branch)?;

    let mut guard = ui::TerminalGuard::new()?;
    let mut terminal = ui::new_terminal()?;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ui::run_app(&mut app, &mut terminal)
    }));

    guard.restore();

    match result {
        Ok(run_result) => {
            if let Err(ref err) = run_result {
                logging::log_error(err);
            }
            run_result
        }
        Err(panic) => {
            let message = logging::panic_message(panic);
            logging::log_panic(&message);
            eprintln!("prdiff crashed: {message}");
            eprintln!("Set RUST_BACKTRACE=1 to see a backtrace.");
            Err(anyhow::anyhow!("panic: {message}"))
        }
    }
}
