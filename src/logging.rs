use std::env;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

static LOG_FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
static TRACE_MOUSE: OnceLock<bool> = OnceLock::new();

pub fn init_logging() {
    let Ok(path) = env::var("PRDIFF_LOG") else {
        return;
    };

    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        eprintln!("prdiff: failed to open PRDIFF_LOG file");
        return;
    };

    let _ = writeln!(file, "=== prdiff start ===");
    let _ = LOG_FILE.set(Mutex::new(file));

    std::panic::set_hook(Box::new(|info| {
        log_line("=== prdiff panic ===");
        log_line(&format!("{info}"));
        let bt = std::backtrace::Backtrace::capture();
        log_line(&format!("{bt}"));
    }));
}

pub fn init_tracing() {
    let enabled = env::var("PRDIFF_TRACE_MOUSE").is_ok();
    let _ = TRACE_MOUSE.set(enabled);
    if enabled {
        log_line("mouse tracing enabled");
    }
}

pub fn trace_mouse(event: &crossterm::event::MouseEvent, in_tree: bool, in_diff: bool) {
    if TRACE_MOUSE.get().copied().unwrap_or(false) {
        log_line(&format!(
            "mouse kind={:?} col={} row={} in_tree={} in_diff={}",
            event.kind, event.column, event.row, in_tree, in_diff
        ));
    }
}

pub fn log_error(err: &anyhow::Error) {
    log_line("=== prdiff error ===");
    log_line(&format!("{err:?}"));
}

pub fn log_panic(message: &str) {
    log_line("=== prdiff panic ===");
    log_line(message);
    let bt = std::backtrace::Backtrace::capture();
    log_line(&format!("{bt}"));
}

fn log_line(msg: &str) {
    let Some(file) = LOG_FILE.get() else {
        return;
    };
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "{msg}");
        let _ = file.flush();
    }
}

#[allow(dead_code)]
pub fn log_debug(msg: &str) {
    log_line(&format!("[DEBUG] {msg}"));
}

pub fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}
