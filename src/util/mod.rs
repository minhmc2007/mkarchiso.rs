pub mod cmd;
pub mod hash;

/// Format a Unix timestamp as UTC `YYYY-MM-DD-HH-MM-SS-00`.
///
/// This matches the string blkid reports as the ISO9660 volume UUID and the
/// format upstream mkarchiso substitutes into `%ARCHISO_UUID%`.
pub fn iso_uuid(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86400);
    let secs_of_day = epoch_secs.rem_euclid(86400);

    // Howard Hinnant's civil_from_days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;

    format!("{:04}-{:02}-{:02}-{:02}-{:02}-{:02}-00", y, m, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::iso_uuid;

    #[test]
    fn formats_epoch_zero() {
        assert_eq!(iso_uuid(0), "1970-01-01-00-00-00-00");
    }

    #[test]
    fn formats_known_timestamp() {
        // 2026-09-22 14:36:40 UTC
        assert_eq!(iso_uuid(1790087800), "2026-09-22-14-36-40-00");
    }
}
