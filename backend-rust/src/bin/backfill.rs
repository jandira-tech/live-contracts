mod args {
    use std::path::PathBuf;

    use anyhow::{Result, bail};

    #[derive(Debug, PartialEq)]
    pub struct BackfillArgs {
        pub output: PathBuf,
        pub from_zip: Option<PathBuf>,
        pub threads: Option<usize>,
    }

    #[derive(Debug, PartialEq)]
    pub enum ParseResult {
        Args(BackfillArgs),
        ShowHelp,
    }

    pub fn parse_args(argv: &[String]) -> Result<ParseResult> {
        let mut output: Option<PathBuf> = None;
        let mut from_zip: Option<PathBuf> = None;
        let mut threads: Option<usize> = None;

        let mut i = 0usize;
        while i < argv.len() {
            match argv[i].as_str() {
                "--help" | "-h" => return Ok(ParseResult::ShowHelp),
                "--output" | "-o" => {
                    i += 1;
                    let val = argv
                        .get(i)
                        .ok_or_else(|| anyhow::anyhow!("--output requires a value"))?;
                    output = Some(PathBuf::from(val));
                }
                "--from-zip" => {
                    i += 1;
                    let val = argv
                        .get(i)
                        .ok_or_else(|| anyhow::anyhow!("--from-zip requires a value"))?;
                    from_zip = Some(PathBuf::from(val));
                }
                "--threads" => {
                    i += 1;
                    let val = argv
                        .get(i)
                        .ok_or_else(|| anyhow::anyhow!("--threads requires a value"))?;
                    let n: usize = val
                        .parse()
                        .map_err(|_| anyhow::anyhow!("--threads must be a positive integer"))?;
                    threads = Some(n);
                }
                flag if flag.starts_with('-') => {
                    bail!("unknown flag: {}", flag);
                }
                other => {
                    bail!("unexpected positional argument: {}", other);
                }
            }
            i += 1;
        }

        let output = output.ok_or_else(|| anyhow::anyhow!("--output <path> is required"))?;

        Ok(ParseResult::Args(BackfillArgs {
            output,
            from_zip,
            threads,
        }))
    }

    pub fn usage() -> &'static str {
        "Usage: backfill --output <path> [--from-zip <path>] [--threads <n>]

Options:
  -o, --output <path>   Output Parquet file path (required)
  --from-zip <path>     Use a local submissions.zip instead of downloading from SEC
  --threads <n>         Number of worker threads (default: 6)
  -h, --help            Print this help and exit"
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn argv(s: &[&str]) -> Vec<String> {
            s.iter().map(|v| v.to_string()).collect()
        }

        #[test]
        fn missing_output_is_error() {
            let result = parse_args(&argv(&[]));
            assert!(result.is_err(), "expected error when --output is missing");
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("--output"),
                "error should mention --output, got: {msg}"
            );
        }

        #[test]
        fn output_only() {
            let result = parse_args(&argv(&["--output", "/tmp/out.parquet"])).unwrap();
            assert_eq!(
                result,
                ParseResult::Args(BackfillArgs {
                    output: PathBuf::from("/tmp/out.parquet"),
                    from_zip: None,
                    threads: None,
                })
            );
        }

        #[test]
        fn short_flag_output() {
            let result = parse_args(&argv(&["-o", "/tmp/out.parquet"])).unwrap();
            assert_eq!(
                result,
                ParseResult::Args(BackfillArgs {
                    output: PathBuf::from("/tmp/out.parquet"),
                    from_zip: None,
                    threads: None,
                })
            );
        }

        #[test]
        fn from_zip_some() {
            let result = parse_args(&argv(&[
                "--output",
                "/tmp/out.parquet",
                "--from-zip",
                "/data/submissions.zip",
            ]))
            .unwrap();
            assert_eq!(
                result,
                ParseResult::Args(BackfillArgs {
                    output: PathBuf::from("/tmp/out.parquet"),
                    from_zip: Some(PathBuf::from("/data/submissions.zip")),
                    threads: None,
                })
            );
        }

        #[test]
        fn from_zip_none_when_not_provided() {
            let result = parse_args(&argv(&["--output", "/tmp/out.parquet"])).unwrap();
            if let ParseResult::Args(a) = result {
                assert!(a.from_zip.is_none());
            } else {
                panic!("expected Args variant");
            }
        }

        #[test]
        fn threads_some() {
            let result = parse_args(&argv(&[
                "--output",
                "/tmp/out.parquet",
                "--threads",
                "4",
            ]))
            .unwrap();
            assert_eq!(
                result,
                ParseResult::Args(BackfillArgs {
                    output: PathBuf::from("/tmp/out.parquet"),
                    from_zip: None,
                    threads: Some(4),
                })
            );
        }

        #[test]
        fn threads_none_when_not_provided() {
            let result = parse_args(&argv(&["--output", "/tmp/out.parquet"])).unwrap();
            if let ParseResult::Args(a) = result {
                assert!(a.threads.is_none());
            } else {
                panic!("expected Args variant");
            }
        }

        #[test]
        fn threads_invalid_value_is_error() {
            let result =
                parse_args(&argv(&["--output", "/tmp/out.parquet", "--threads", "abc"]));
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(msg.contains("--threads"), "got: {msg}");
        }

        #[test]
        fn unknown_flag_is_error() {
            let result =
                parse_args(&argv(&["--output", "/tmp/out.parquet", "--unknown"]));
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(msg.contains("unknown flag"), "got: {msg}");
        }

        #[test]
        fn help_long_returns_show_help() {
            let result = parse_args(&argv(&["--help"])).unwrap();
            assert_eq!(result, ParseResult::ShowHelp);
        }

        #[test]
        fn help_short_returns_show_help() {
            let result = parse_args(&argv(&["-h"])).unwrap();
            assert_eq!(result, ParseResult::ShowHelp);
        }

        #[test]
        fn help_before_output_still_shows_help() {
            let result =
                parse_args(&argv(&["--help", "--output", "/tmp/x.parquet"])).unwrap();
            assert_eq!(result, ParseResult::ShowHelp);
        }

        #[test]
        fn all_flags_together() {
            let result = parse_args(&argv(&[
                "--output",
                "/tmp/out.parquet",
                "--from-zip",
                "/data/submissions.zip",
                "--threads",
                "8",
            ]))
            .unwrap();
            assert_eq!(
                result,
                ParseResult::Args(BackfillArgs {
                    output: PathBuf::from("/tmp/out.parquet"),
                    from_zip: Some(PathBuf::from("/data/submissions.zip")),
                    threads: Some(8),
                })
            );
        }

        #[test]
        fn output_flag_missing_value_is_error() {
            let result = parse_args(&argv(&["--output"]));
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(msg.contains("--output"), "got: {msg}");
        }
    }
}

use std::time::Instant;

use args::{ParseResult, parse_args, usage};
use secinfra::{construct_submissions_metadata, construct_submissions_metadata_from_zip};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let argv: Vec<String> = std::env::args().skip(1).collect();

    let parsed = match parse_args(&argv) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}\n\n{}", usage());
            std::process::exit(2);
        }
    };

    let backfill_args = match parsed {
        ParseResult::ShowHelp => {
            println!("{}", usage());
            std::process::exit(0);
        }
        ParseResult::Args(a) => a,
    };

    let t0 = Instant::now();

    let result = if let Some(zip_path) = backfill_args.from_zip {
        let output = backfill_args.output.clone();
        let threads = backfill_args.threads;
        tokio::task::spawn_blocking(move || {
            construct_submissions_metadata_from_zip(output, zip_path, None, threads)
        })
        .await
        .expect("spawn_blocking panicked")
    } else {
        construct_submissions_metadata(backfill_args.output, None, None, backfill_args.threads)
            .await
    };

    match result {
        Ok(stats) => {
            let elapsed = t0.elapsed();
            tracing::info!(
                files_processed = stats.files_processed,
                filings_written = stats.filings_written,
                batches_written = stats.batches_written,
                files_skipped = stats.files_skipped,
                elapsed_secs = elapsed.as_secs_f64(),
                "backfill complete"
            );
        }
        Err(e) => {
            tracing::error!("backfill failed: {e:#}");
            std::process::exit(1);
        }
    }
}
