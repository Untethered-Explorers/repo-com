#![forbid(unsafe_code)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use repo_com_performance::{Fixture, HarnessConfig, ensure_final_binary, report_json, run_harness};

fn main() -> ExitCode {
    match execute() {
        Ok((report, passed)) => {
            let json = match report_json(&report) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("repo-com-performance: {error}");
                    return ExitCode::from(1);
                }
            };
            println!("{json}");
            if passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            eprintln!("repo-com-performance: {error}");
            ExitCode::from(1)
        }
    }
}

fn execute()
-> Result<(repo_com_performance::PerformanceReport, bool), repo_com_performance::HarnessError> {
    let mut binary = None;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--binary" => {
                let value = args.next().ok_or_else(|| {
                    repo_com_performance::HarnessError::new("--binary requires a path")
                })?;
                binary = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                println!(
                    "Usage: repo-com-performance [--binary PATH]\n\nRuns the REL-PERF-1 token-free local command harness and writes JSON evidence to stdout."
                );
                std::process::exit(0);
            }
            other => {
                return Err(repo_com_performance::HarnessError::new(format!(
                    "unknown argument: {other}"
                )));
            }
        }
    }

    let binary = match binary {
        Some(path) => path,
        None => ensure_final_binary()?,
    };
    let fixture = Fixture::create()?;
    let report = run_harness(&binary, &fixture, &HarnessConfig::default())?;
    let passed = report.passed;
    Ok((report, passed))
}
