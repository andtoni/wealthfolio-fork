use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

use crate::error::SpendingError;
use crate::settings::{BudgetCadence, BudgetCycleSettings};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetPeriod {
    pub key: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

pub fn period_for_date(date: NaiveDate, cycle: &BudgetCycleSettings) -> Result<BudgetPeriod> {
    let anchor = anchor_date(cycle)?;
    let (start, end) = match cycle.cadence {
        BudgetCadence::Weekly => fixed_day_period(date, anchor, 7),
        BudgetCadence::Biweekly => fixed_day_period(date, anchor, 14),
        BudgetCadence::Monthly => monthly_period(date, anchor)?,
    };
    Ok(BudgetPeriod {
        key: start.format("%Y-%m-%d").to_string(),
        start,
        end,
    })
}

pub fn period_for_key(key: &str, cycle: &BudgetCycleSettings) -> Result<BudgetPeriod> {
    let start = parse_period_key(key)?;
    let period = period_for_date(start, cycle)?;
    if period.start != start {
        return Err(invalid_period_key());
    }
    Ok(period)
}

pub fn current_period_key(timezone: &str, cycle: &BudgetCycleSettings) -> Result<String> {
    let local =
        wealthfolio_core::utils::time_utils::activity_date_in_user_timezone(Utc::now(), timezone);
    Ok(period_for_date(local, cycle)?.key)
}

pub fn shift_period_key(key: &str, periods: i32, cycle: &BudgetCycleSettings) -> Result<String> {
    let period = period_for_key(key, cycle)?;
    let anchor = anchor_date(cycle)?;
    let shifted = match cycle.cadence {
        BudgetCadence::Weekly => period.start + Duration::days(i64::from(periods) * 7),
        BudgetCadence::Biweekly => period.start + Duration::days(i64::from(periods) * 14),
        BudgetCadence::Monthly => {
            let index = month_index(period.start.year(), period.start.month())
                - month_index(anchor.year(), anchor.month());
            monthly_occurrence(anchor, index + periods)?
        }
    };
    Ok(shifted.format("%Y-%m-%d").to_string())
}

pub fn period_keys_between(
    start_key: &str,
    end_key: &str,
    cycle: &BudgetCycleSettings,
) -> Result<Vec<String>> {
    let start = period_for_key(start_key, cycle)?;
    let end = period_for_key(end_key, cycle)?;
    if start.start > end.start {
        return Ok(Vec::new());
    }

    let mut keys = Vec::new();
    let mut key = start.key;
    while key <= end.key {
        keys.push(key.clone());
        key = shift_period_key(&key, 1, cycle)?;
    }
    Ok(keys)
}

pub fn local_period_bounds_utc(
    key: &str,
    timezone: &str,
    cycle: &BudgetCycleSettings,
) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let period = period_for_key(key, cycle)?;
    let next_start = period.end + Duration::days(1);
    let tz = wealthfolio_core::utils::time_utils::parse_user_timezone_or_default(timezone);
    let start = first_valid_instant_of_local_day(period.start, tz)?;
    let end_exclusive = first_valid_instant_of_local_day(next_start, tz)?;
    Ok((start, end_exclusive - Duration::nanoseconds(1)))
}

fn first_valid_instant_of_local_day(
    date: NaiveDate,
    timezone: chrono_tz::Tz,
) -> Result<DateTime<Utc>> {
    let midnight = date.and_hms_opt(0, 0, 0).ok_or_else(invalid_period_key)?;
    for minute in 0..=2_880 {
        if let Some(local) = timezone
            .from_local_datetime(&(midnight + Duration::minutes(minute)))
            .earliest()
        {
            return Ok(local.with_timezone(&Utc));
        }
    }
    Err(invalid_period_key())
}

fn fixed_day_period(date: NaiveDate, anchor: NaiveDate, days: i64) -> (NaiveDate, NaiveDate) {
    let index = (date - anchor).num_days().div_euclid(days);
    let start = anchor + Duration::days(index * days);
    (start, start + Duration::days(days - 1))
}

fn monthly_period(date: NaiveDate, anchor: NaiveDate) -> Result<(NaiveDate, NaiveDate)> {
    let mut index =
        month_index(date.year(), date.month()) - month_index(anchor.year(), anchor.month());
    let candidate = monthly_occurrence(anchor, index)?;
    if candidate > date {
        index -= 1;
    }
    let start = monthly_occurrence(anchor, index)?;
    let next = monthly_occurrence(anchor, index + 1)?;
    Ok((start, next - Duration::days(1)))
}

fn monthly_occurrence(anchor: NaiveDate, offset: i32) -> Result<NaiveDate> {
    let absolute = month_index(anchor.year(), anchor.month()) + offset;
    let year = absolute.div_euclid(12);
    let month = absolute.rem_euclid(12) as u32 + 1;
    let last_day = days_in_month(year, month)?;
    let anchor_is_month_end = anchor.day() == days_in_month(anchor.year(), anchor.month())?;
    let day = if anchor_is_month_end {
        last_day
    } else {
        anchor.day().min(last_day)
    };
    NaiveDate::from_ymd_opt(year, month, day).ok_or_else(invalid_period_key)
}

fn month_index(year: i32, month: u32) -> i32 {
    year * 12 + month as i32 - 1
}

fn days_in_month(year: i32, month: u32) -> Result<u32> {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next = NaiveDate::from_ymd_opt(next_year, next_month, 1).ok_or_else(invalid_period_key)?;
    Ok((next - Duration::days(1)).day())
}

fn anchor_date(cycle: &BudgetCycleSettings) -> Result<NaiveDate> {
    cycle.parsed_anchor_date().ok_or_else(invalid_period_key)
}

fn parse_period_key(key: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(key, "%Y-%m-%d").map_err(|_| invalid_period_key())
}

fn invalid_period_key() -> anyhow::Error {
    SpendingError::InvalidInput {
        message: "Invalid budget period key".to_string(),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;

    fn cycle(cadence: BudgetCadence, anchor_date: &str) -> BudgetCycleSettings {
        BudgetCycleSettings {
            cadence,
            anchor_date: anchor_date.to_string(),
        }
    }

    #[test]
    fn weekly_and_biweekly_cycles_follow_the_anchor() {
        let weekly = cycle(BudgetCadence::Weekly, "2026-01-07");
        assert_eq!(
            period_for_date(NaiveDate::from_ymd_opt(2026, 1, 6).unwrap(), &weekly).unwrap(),
            BudgetPeriod {
                key: "2025-12-31".to_string(),
                start: NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
                end: NaiveDate::from_ymd_opt(2026, 1, 6).unwrap(),
            }
        );

        let biweekly = cycle(BudgetCadence::Biweekly, "2026-01-01");
        let period =
            period_for_date(NaiveDate::from_ymd_opt(2026, 1, 29).unwrap(), &biweekly).unwrap();
        assert_eq!(period.key, "2026-01-29");
        assert_eq!(period.end, NaiveDate::from_ymd_opt(2026, 2, 11).unwrap());
    }

    #[test]
    fn monthly_cycle_clamps_high_days_and_preserves_month_end_anchor() {
        let day_30 = cycle(BudgetCadence::Monthly, "2026-01-30");
        let february =
            period_for_date(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(), &day_30).unwrap();
        assert_eq!(
            february.start,
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        );
        assert_eq!(february.end, NaiveDate::from_ymd_opt(2026, 3, 29).unwrap());

        let month_end = cycle(BudgetCadence::Monthly, "2024-02-29");
        let non_leap =
            period_for_date(NaiveDate::from_ymd_opt(2025, 2, 28).unwrap(), &month_end).unwrap();
        assert_eq!(
            non_leap.start,
            NaiveDate::from_ymd_opt(2025, 2, 28).unwrap()
        );
        assert_eq!(non_leap.end, NaiveDate::from_ymd_opt(2025, 3, 30).unwrap());
    }

    #[test]
    fn legacy_default_remains_monthly_from_the_25th() {
        let legacy = BudgetCycleSettings::default();
        let period =
            period_for_date(NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(), &legacy).unwrap();
        assert_eq!(period.key, "2026-07-25");
        assert_eq!(period.end, NaiveDate::from_ymd_opt(2026, 8, 24).unwrap());
    }

    #[test]
    fn local_bounds_honor_dst_and_inclusive_end() {
        let monthly = cycle(BudgetCadence::Monthly, "2026-01-25");
        let (start, end) =
            local_period_bounds_utc("2026-02-25", "America/Toronto", &monthly).unwrap();
        assert_eq!(start.to_rfc3339(), "2026-02-25T05:00:00+00:00");
        assert_eq!(end.to_rfc3339(), "2026-03-25T03:59:59.999999999+00:00");
    }

    #[test]
    fn skipped_local_midnight_uses_the_first_valid_instant() {
        let weekly = cycle(BudgetCadence::Weekly, "2026-08-30");
        let (start, _) = local_period_bounds_utc("2026-09-06", "America/Santiago", &weekly)
            .expect("DST-skipped midnight must not invalidate the cycle");
        let local = start.with_timezone(&chrono_tz::America::Santiago);

        assert_eq!(
            local.date_naive(),
            NaiveDate::from_ymd_opt(2026, 9, 6).unwrap()
        );
        assert_eq!(local.hour(), 1);
    }

    #[test]
    fn rejects_dates_that_are_not_cycle_starts() {
        let weekly = cycle(BudgetCadence::Weekly, "2026-01-01");
        assert!(period_for_key("2026-01-02", &weekly).is_err());
    }
}
