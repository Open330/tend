//! Parse Claude Code's weekly reset notice using the timezone it displays.
use jiff::{civil::Date, tz::TimeZone, Timestamp};
use regex::Regex;
use std::sync::LazyLock;
use time::OffsetDateTime;

static WEEKLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(?:⎿\s*)?(?:you(?:['’]ve| have) hit your\s+weekly\s+limit|weekly limit reached)\b")
        .expect("weekly limit regex")
});
static RESET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?ix)^\s*[·.]?\s*(?:your\s+limit\s+will\s+)?resets?(?:\s+at)?\s+(?:([a-z]{3})\s+(\d{1,2})\s+at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)\s*\(([^()\s]+)\)")
        .expect("weekly reset regex")
});

pub(crate) fn is_weekly(message: &str) -> bool {
    WEEKLY.is_match(message)
}

pub(crate) fn reset_at(message: &str, observed_at: OffsetDateTime) -> Option<OffsetDateTime> {
    let notice = WEEKLY.find(message)?;
    let fields = RESET.captures(&message[notice.end()..])?;
    let zone = TimeZone::get(&fields[6]).ok()?;
    let observed = Timestamp::from_second(observed_at.unix_timestamp()).ok()?;
    let local = zone.to_datetime(observed);
    let hour = fields[3]
        .parse::<i8>()
        .ok()
        .filter(|hour| (1..=12).contains(hour))?;
    let hour = hour % 12
        + if fields[5].eq_ignore_ascii_case("pm") {
            12
        } else {
            0
        };
    let minute = fields
        .get(4)
        .map_or(Some(0), |m| m.as_str().parse::<i8>().ok())?;
    if !(0..=59).contains(&minute) {
        return None;
    }
    let explicit_date = fields.get(1);
    let mut date = if let Some(month) = explicit_date {
        let day = fields[2].parse::<i8>().ok()?;
        let month = [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ]
        .iter()
        .position(|s| s.eq_ignore_ascii_case(month.as_str()))?;
        let month = i8::try_from(month + 1).ok()?;
        // The notice omits the year. Find the nearby civil date, including
        // next January when a weekly window crosses New Year's Eve.
        [local.year(), local.year() + 1, local.year() - 1]
            .into_iter()
            .filter_map(|year| Date::new(year, month, day).ok())
            .find(|date| {
                date.since(local.date())
                    .is_ok_and(|span| span.get_days().abs() <= 8)
            })?
    } else {
        local.date()
    };
    let to_timestamp = |date: Date| {
        let local = date.at(hour, minute, 0, 0);
        zone.to_ambiguous_timestamp(local).later().ok()
    };
    let mut reset = to_timestamp(date)?;
    // A time-only notice observed after the displayed minute means the
    // next occurrence. The caller always uses the observation time here.
    if explicit_date.is_none() && observed.as_second() - reset.as_second() >= 60 {
        date = date.tomorrow().ok()?;
        reset = to_timestamp(date)?;
    }
    OffsetDateTime::from_unix_timestamp(reset.as_second()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn reads_the_weekly_notice_and_its_displayed_timezone() {
        let notice = "Usage limit reached again · set to continue automatically at 4pm\nYou've hit your weekly limit · resets 4pm (Asia/Seoul)\n/upgrade or /usage-credits";
        assert_eq!(
            reset_at(notice, datetime!(2026-11-26 11:00 +09:00)),
            Some(datetime!(2026-11-26 07:00 UTC))
        );
        assert_eq!(
            reset_at(notice, datetime!(2026-11-26 16:00:30 +09:00)),
            Some(datetime!(2026-11-26 07:00 UTC))
        );
        assert_eq!(
            reset_at(notice, datetime!(2026-11-26 16:01 +09:00)),
            Some(datetime!(2026-11-27 07:00 UTC))
        );
        assert_eq!(
            reset_at(
                "You've hit your\n weekly limit ·\n resets Jan 1 at\n 12:05am\n (Asia/Seoul)",
                datetime!(2026-12-31 18:00 +09:00)
            ),
            Some(datetime!(2026-12-31 15:05 UTC))
        );
    }

    #[test]
    fn unreadable_resets_cannot_supply_a_deadline() {
        for suffix in [
            "4pm",
            "4pm (Unknown/Zone)",
            "13pm (Asia/Seoul)",
            "4:91pm (Asia/Seoul)",
        ] {
            let notice = format!("You’ve hit your weekly limit · resets {suffix}");
            assert!(is_weekly(&notice));
            assert_eq!(reset_at(&notice, datetime!(2026-11-26 11:00 +09:00)), None);
        }
    }
}
