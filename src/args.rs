use crate::theme::ThemeMode;
use anyhow::Result;
use std::env;

pub struct Args {
    pub base_branch: Option<String>,
    pub theme: Option<ThemeMode>,
}

fn print_usage() {
    eprintln!("prdiff - Terminal PR diff viewer");
    eprintln!();
    eprintln!("Usage: prdiff [OPTIONS] [BASE_BRANCH]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -b, --base <BRANCH>    Base branch to diff against");
    eprintln!("  -t, --theme <THEME>    Color theme: light or dark (default: dark)");
    eprintln!("  -h, --help             Show this help message");
    eprintln!();
    eprintln!("Environment:");
    eprintln!("  PRDIFF_THEME           Color theme (overrides --theme flag)");
    eprintln!();
    eprintln!("If no base branch specified, auto-detects upstream/develop/main/master");
}

pub fn parse_args() -> Result<Args> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut base_branch = None;
    let mut theme = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-b" | "--base" => {
                i += 1;
                if i >= args.len() {
                    anyhow::bail!("--base requires a branch name");
                }
                base_branch = Some(args[i].clone());
            }
            "-t" | "--theme" => {
                i += 1;
                if i >= args.len() {
                    anyhow::bail!("--theme requires a value: light or dark");
                }
                match ThemeMode::from_str(&args[i]) {
                    Some(mode) => theme = Some(mode),
                    None => anyhow::bail!("Invalid theme '{}': must be 'light' or 'dark'", args[i]),
                }
            }
            arg if arg.starts_with('-') => {
                anyhow::bail!("Unknown option: {arg}");
            }
            arg => {
                if base_branch.is_none() {
                    base_branch = Some(arg.to_string());
                } else {
                    anyhow::bail!("Unexpected argument: {arg}");
                }
            }
        }
        i += 1;
    }

    Ok(Args { base_branch, theme })
}
