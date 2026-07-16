use chrono::TimeZone;

pub(crate) fn naive_local_to_utc(v: chrono::NaiveDateTime) -> chrono::DateTime<chrono::Utc> {
    match chrono::Local.from_local_datetime(&v) {
        chrono::LocalResult::Single(dt) => dt.with_timezone(&chrono::Utc),
        chrono::LocalResult::Ambiguous(dt, _) => dt.with_timezone(&chrono::Utc),
        chrono::LocalResult::None => panic!("invalid local NaiveDateTime: {v}"),
    }
}

pub(crate) fn utc_to_naive_local(v: chrono::DateTime<chrono::Utc>) -> chrono::NaiveDateTime {
    v.with_timezone(&chrono::Local).naive_local()
}

#[cfg(test)]
mod tests {
    use super::{naive_local_to_utc, utc_to_naive_local};
    use crate::model::{FromValue, Value};
    use chrono::{NaiveDate, TimeZone};

    fn sample_local_time() -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 1, 15)
            .unwrap()
            .and_hms_micro_opt(14, 7, 0, 123_456)
            .unwrap()
    }

    #[test]
    fn naive_datetime_value_conversion_preserves_local_wall_time() {
        let local_naive = sample_local_time();
        let stored_utc = match Value::from(local_naive) {
            Value::DateTime(value) => value,
            _ => panic!("expected DateTime value"),
        };

        assert_eq!(utc_to_naive_local(stored_utc), local_naive);
        assert_eq!(
            chrono::NaiveDateTime::from_value(&Value::DateTime(stored_utc)).unwrap(),
            local_naive
        );
    }

    #[test]
    fn naive_datetime_value_conversion_uses_local_timezone() {
        let local_naive = sample_local_time();
        let local_utc = naive_local_to_utc(local_naive);
        let direct_utc = chrono::Utc.from_utc_datetime(&local_naive);

        assert_eq!(
            local_utc.with_timezone(&chrono::Local).naive_local(),
            local_naive
        );
        if local_utc
            .with_timezone(&chrono::Local)
            .offset()
            .local_minus_utc()
            != 0
        {
            assert_ne!(local_utc, direct_utc);
        }
    }
}
