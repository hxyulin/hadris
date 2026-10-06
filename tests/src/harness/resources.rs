use std::process::{Command, Output};

/// Runs a fresh process through the platform's time utility. RSS includes
/// startup, setup, warm-up and verification, not only the timed operation.
pub fn run_with_peak_rss(command: &mut Command) -> Result<(Output, u64), String> {
    let mut timer = Command::new("/usr/bin/time");
    if cfg!(target_os = "macos") {
        timer.arg("-l");
    } else if cfg!(target_os = "linux") {
        timer.args(["-f", "HADRIS_PEAK_RSS_KIB=%M"]);
    } else {
        return Err("RSS measurement requires macOS time or GNU time on Linux".into());
    }
    timer.arg(command.get_program()).args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => {
                timer.env(key, value);
            }
            None => {
                timer.env_remove(key);
            }
        }
    }
    if let Some(directory) = command.get_current_dir() {
        timer.current_dir(directory);
    }
    let output = timer
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "resource worker failed: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let rss = parse_peak_rss(&stderr, cfg!(target_os = "macos"))?;
    Ok((output, rss))
}

fn parse_peak_rss(text: &str, macos: bool) -> Result<u64, String> {
    let value = text
        .lines()
        .find_map(|line| {
            if macos {
                line.trim()
                    .strip_suffix("maximum resident set size")
                    .map(str::trim)
            } else {
                line.strip_prefix("HADRIS_PEAK_RSS_KIB=")
            }
        })
        .ok_or("time output has no peak RSS")?;
    let value: u64 = value.parse().map_err(|_| "invalid peak RSS")?;
    value
        .checked_mul(if macos { 1 } else { 1024 })
        .ok_or_else(|| "peak RSS overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_units_are_normalized_and_missing_measurements_fail() {
        assert_eq!(
            parse_peak_rss("  8192  maximum resident set size\n", true).unwrap(),
            8192
        );
        assert_eq!(
            parse_peak_rss("diagnostic\nHADRIS_PEAK_RSS_KIB=8\n", false).unwrap(),
            8192
        );
        for text in [
            "",
            "HADRIS_PEAK_RSS_KIB=unknown",
            "HADRIS_PEAK_RSS_KIB=18446744073709551615",
        ] {
            assert!(parse_peak_rss(text, false).is_err());
        }
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn failed_workers_cannot_produce_valid_samples() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        assert!(
            run_with_peak_rss(&mut command)
                .unwrap_err()
                .contains("resource worker failed")
        );
    }
}
