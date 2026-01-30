use anyhow::Result;
use std::env;

fn print_usage() {
    eprintln!("prdiff - Terminal PR diff viewer");
    eprintln!();
    eprintln!("Usage: prdiff [OPTIONS] [BASE_BRANCH]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -b, --base <BRANCH>  Base branch to diff against");
    eprintln!("  -h, --help           Show this help message");
    eprintln!();
    eprintln!("If no base branch specified, auto-detects upstream/develop/main/master");
}

pub fn parse_args() -> Result<Option<String>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut base_branch = None;
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

    Ok(base_branch)
}
