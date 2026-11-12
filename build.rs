// Sets LOGIX_ENGINE_BUILD_VERSION, the engine version reported by get_engine_info
// and --version, as people see it: yyyy.mm.dd (the Open Logix versioning rule).
// Release builds pass the release tag (yyyy-mm-dd) in the LOGIX_ENGINE_BUILD_VERSION
// env var. Other builds use today's UTC date.

use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-env-changed=LOGIX_ENGINE_BUILD_VERSION");
    let version = match std::env::var("LOGIX_ENGINE_BUILD_VERSION") {
        Ok(v) if !v.is_empty() => {
            if !is_release_tag(&v) {
                panic!("LOGIX_ENGINE_BUILD_VERSION must be a release tag, yyyy-mm-dd, got {v:?}");
            }
            v.replace('-', ".")
        }
        _ => today_utc(),
    };
    println!("cargo:rustc-env=LOGIX_ENGINE_BUILD_VERSION={version}");
}

fn is_release_tag(v: &str) -> bool {
    let b = v.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn today_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before 1970")
        .as_secs();
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}.{m:02}.{d:02}")
}

// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}
