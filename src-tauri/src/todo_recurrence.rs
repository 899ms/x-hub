//! 周期待办规则引擎（**规则的唯一实现**）。
//!
//! 滚动（§5.1）与日历虚拟展开（§5.4）共用本模块：前端只消费
//! `expand_todo_occurrences` 命令的返回结果渲染，不重写规则——规则只存在一份，
//! 从根上不存在「两端漂移」。
//!
//! 时间语义与前端一致：毫秒时间戳按**本地时区**落到「日」，重复时保留首次排期的
//! 时分（§5.1 滚动只推日期）。

use crate::models::{RepeatRule, Todo};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Weekday};

/// 迭代上限：规则异常时防死循环（§5.1）
pub const MAX_ITER: usize = 1000;

pub fn is_recurring(t: &Todo) -> bool {
    t.repeat_mode != "once"
}

/// 位掩码 bit0=周一 … bit6=周日
fn mask_has(mask: i64, wd: Weekday) -> bool {
    mask & (1 << wd.num_days_from_monday()) != 0
}

fn weekday_offsets(mask: i64) -> Vec<u32> {
    (0..7u32).filter(|i| mask & (1 << i) != 0).collect()
}

/// 本地「日 + 时分」→ 毫秒时间戳。夏令时歧义取较早的一侧，缺失时刻（春季跳变）取 None。
fn at_time(date: NaiveDate, time: NaiveTime) -> Option<i64> {
    Local
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .map(|dt| dt.timestamp_millis())
}

fn weekday_from_index(i: u32) -> Weekday {
    match i {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

/// 取某月最后一天
fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.day())
        .unwrap_or(28)
}

/// 某月第 n 个（n = 1..5；-1 = 最后一个）指定星期几
fn nth_weekday_of_month(year: i32, month: u32, weekday: Weekday, nth: i64) -> Option<NaiveDate> {
    if nth == -1 {
        let last = last_day_of_month(year, month);
        let d = NaiveDate::from_ymd_opt(year, month, last)?;
        let back = (d.weekday().num_days_from_monday() as i64
            - weekday.num_days_from_monday() as i64)
            .rem_euclid(7);
        return d.checked_sub_signed(Duration::days(back));
    }
    if nth < 1 {
        return None;
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let fwd = (weekday.num_days_from_monday() as i64
        - first.weekday().num_days_from_monday() as i64)
        .rem_euclid(7)
        + (nth - 1) * 7;
    let day = 1 + fwd as u32;
    if day > last_day_of_month(year, month) {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

fn month_anchor(year: i32, month: u32, rule: &RepeatRule) -> Option<NaiveDate> {
    match (rule.month_nth, rule.month_day) {
        (Some(nth), _) => {
            let mask = rule.weekdays.unwrap_or(0);
            let offsets = weekday_offsets(mask);
            let wd = weekday_from_index(*offsets.first()?);
            nth_weekday_of_month(year, month, wd, nth)
        }
        (None, Some(day)) => {
            let d = if day == -1 {
                last_day_of_month(year, month)
            } else {
                (day.max(1) as u32).min(last_day_of_month(year, month))
            };
            NaiveDate::from_ymd_opt(year, month, d)
        }
        _ => None,
    }
}

/// 从 base 时刻所在「周」的周一 0 点（仅用于 custom+week 的周计数）
fn week_start_ms(ms: i64) -> Option<i64> {
    let dt = Local.timestamp_millis_opt(ms).single()?;
    let date = dt.date_naive() - Duration::days(dt.weekday().num_days_from_monday() as i64);
    at_time(date, NaiveTime::from_hms_opt(0, 0, 0)?)
}

/// 下一个实例（严格晚于 prev）。规则用尽（如 monthly 无合法日）返回 None。
fn advance(rule: &RepeatRule, prev: i64, base_time: NaiveTime) -> Option<i64> {
    let dt = Local.timestamp_millis_opt(prev).single()?;
    let date = dt.date_naive();
    match rule.mode.as_str() {
        "daily" => at_time(date + Duration::days(1), base_time),
        "weekdays" => {
            let mut d = date + Duration::days(1);
            for _ in 0..7 {
                if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
                    return at_time(d, base_time);
                }
                d += Duration::days(1);
            }
            None
        }
        "weekly" => {
            let mask = rule.weekdays.unwrap_or(0);
            if mask == 0 {
                return at_time(date + Duration::days(7), base_time);
            }
            let mut d = date + Duration::days(1);
            for _ in 0..7 {
                if mask_has(mask, d.weekday()) {
                    return at_time(d, base_time);
                }
                d += Duration::days(1);
            }
            None
        }
        "monthly" => {
            let (ny, nm) = if date.month() == 12 {
                (date.year() + 1, 1)
            } else {
                (date.year(), date.month() + 1)
            };
            month_anchor(ny, nm, rule).and_then(|d| at_time(d, base_time))
        }
        "yearly" => {
            let y = date.year() + 1;
            let m = date.month();
            let day = date.day().min(last_day_of_month(y, m));
            NaiveDate::from_ymd_opt(y, m, day).and_then(|d| at_time(d, base_time))
        }
        "custom" => {
            let every = rule.every.unwrap_or(1).max(1);
            match rule.unit.as_deref().unwrap_or("day") {
                "week" => {
                    let mask = rule.weekdays.unwrap_or(0);
                    if mask == 0 {
                        return at_time(date + Duration::weeks(every), base_time);
                    }
                    // 位掩码 + 每 N 周：逐日找掩码内的星期几，且周序号差是 N 的倍数
                    let base_week = week_start_ms(prev)?;
                    let mut d = date + Duration::days(1);
                    for _ in 0..(every * 7) {
                        if mask_has(mask, d.weekday()) {
                            if let Some(ms) = at_time(d, base_time) {
                                let ws = week_start_ms(ms)?;
                                let weeks = (ws - base_week) / (7 * 24 * 3600 * 1000);
                                if weeks.rem_euclid(every) == 0 {
                                    return Some(ms);
                                }
                            }
                        }
                        d += Duration::days(1);
                    }
                    None
                }
                "month" => {
                    let total = date.year() * 12 + (date.month() as i32 - 1) + every as i32;
                    let (ny, nm) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
                    let day = date.day().min(last_day_of_month(ny, nm));
                    NaiveDate::from_ymd_opt(ny, nm, day).and_then(|d| at_time(d, base_time))
                }
                "year" => {
                    let y = date.year() + every as i32;
                    let m = date.month();
                    let day = date.day().min(last_day_of_month(y, m));
                    NaiveDate::from_ymd_opt(y, m, day).and_then(|d| at_time(d, base_time))
                }
                _ => at_time(date + Duration::days(every), base_time),
            }
        }
        _ => None,
    }
}

/// 上一个实例（严格早于 cur），用于撤销勾选时把 due_at 滚回本轮。
fn retreat(rule: &RepeatRule, cur: i64, base_time: NaiveTime) -> Option<i64> {
    let dt = Local.timestamp_millis_opt(cur).single()?;
    let date = dt.date_naive();
    match rule.mode.as_str() {
        "daily" => at_time(date - Duration::days(1), base_time),
        "weekdays" => {
            let mut d = date - Duration::days(1);
            for _ in 0..7 {
                if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
                    return at_time(d, base_time);
                }
                d -= Duration::days(1);
            }
            None
        }
        "weekly" => {
            let mask = rule.weekdays.unwrap_or(0);
            if mask == 0 {
                return at_time(date - Duration::days(7), base_time);
            }
            let mut d = date - Duration::days(1);
            for _ in 0..7 {
                if mask_has(mask, d.weekday()) {
                    return at_time(d, base_time);
                }
                d -= Duration::days(1);
            }
            None
        }
        "monthly" => {
            let (py, pm) = if date.month() == 1 {
                (date.year() - 1, 12)
            } else {
                (date.year(), date.month() - 1)
            };
            month_anchor(py, pm, rule).and_then(|d| at_time(d, base_time))
        }
        "yearly" => {
            let y = date.year() - 1;
            let m = date.month();
            let day = date.day().min(last_day_of_month(y, m));
            NaiveDate::from_ymd_opt(y, m, day).and_then(|d| at_time(d, base_time))
        }
        "custom" => {
            let every = rule.every.unwrap_or(1).max(1);
            match rule.unit.as_deref().unwrap_or("day") {
                "week" => {
                    let mask = rule.weekdays.unwrap_or(0);
                    if mask == 0 {
                        return at_time(date - Duration::weeks(every), base_time);
                    }
                    let base_week = week_start_ms(cur)?;
                    let mut d = date - Duration::days(1);
                    for _ in 0..(every * 7) {
                        if mask_has(mask, d.weekday()) {
                            if let Some(ms) = at_time(d, base_time) {
                                let ws = week_start_ms(ms)?;
                                let weeks = (base_week - ws) / (7 * 24 * 3600 * 1000);
                                if weeks.rem_euclid(every) == 0 {
                                    return Some(ms);
                                }
                            }
                        }
                        d -= Duration::days(1);
                    }
                    None
                }
                "month" => {
                    let total = date.year() * 12 + (date.month() as i32 - 1) - every as i32;
                    let (py, pm) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
                    let day = date.day().min(last_day_of_month(py, pm));
                    NaiveDate::from_ymd_opt(py, pm, day).and_then(|d| at_time(d, base_time))
                }
                "year" => {
                    let y = date.year() - every as i32;
                    let m = date.month();
                    let day = date.day().min(last_day_of_month(y, m));
                    NaiveDate::from_ymd_opt(y, m, day).and_then(|d| at_time(d, base_time))
                }
                _ => at_time(date - Duration::days(every), base_time),
            }
        }
        _ => None,
    }
}

/// 从 due_at 起算、**严格晚于 from_ms** 的第一个实例时刻（§5.1 滚动用）。
/// 逾期不补历史：连续迭代直到落在 from_ms 之后。
pub fn next_occurrence(rule: &RepeatRule, due_at: i64, from_ms: i64) -> Option<i64> {
    if rule.is_once() {
        return None;
    }
    let base_time = Local.timestamp_millis_opt(due_at).single()?.time();
    let mut prev = due_at;
    for _ in 0..MAX_ITER {
        let next = advance(rule, prev, base_time)?;
        if !within_end(rule, next) {
            return None;
        }
        if next > from_ms {
            return Some(next);
        }
        prev = next;
    }
    None
}

/// 撤销用：严格早于 cur 的实例时刻
pub fn previous_occurrence(rule: &RepeatRule, cur: i64) -> Option<i64> {
    if rule.is_once() {
        return None;
    }
    let base_time = Local.timestamp_millis_opt(cur).single()?.time();
    retreat(rule, cur, base_time)
}

/// 结束条件（until）：超出截止日即结束。count 由调用方按剩余次数截断。
fn within_end(rule: &RepeatRule, at_ms: i64) -> bool {
    match rule.end_mode.as_deref() {
        Some("until") => rule.end_at.map(|end| at_ms <= end).unwrap_or(true),
        _ => true,
    }
}

/// 结束条件是否已用尽（滚动到 next 之前判断，§5.3）
pub fn reached_end(rule: &RepeatRule, next: i64, done_count: i64) -> bool {
    if !within_end(rule, next) {
        return true;
    }
    match rule.end_mode.as_deref() {
        Some("count") => match rule.count {
            Some(total) => done_count + 1 >= total,
            None => false,
        },
        _ => false,
    }
}

/// 展开 [from_ms, to_ms] 内的**虚拟实例**（不含库里那一行当前实例本身）。
/// count 结束条件按剩余次数（count - done_count）截断。
pub fn expand_occurrences(
    rule: &RepeatRule,
    due_at: i64,
    done_count: i64,
    from_ms: i64,
    to_ms: i64,
) -> Vec<i64> {
    let mut out = Vec::new();
    if rule.is_once() || to_ms < from_ms {
        return out;
    }
    let Some(base_time) = Local.timestamp_millis_opt(due_at).single().map(|d| d.time()) else {
        return out;
    };
    let mut remaining = match rule.end_mode.as_deref() {
        Some("count") => match rule.count {
            Some(total) => (total - done_count).max(0),
            None => i64::MAX,
        },
        _ => i64::MAX,
    };
    let mut prev = due_at;
    for _ in 0..MAX_ITER {
        let Some(next) = advance(rule, prev, base_time) else {
            break;
        };
        if !within_end(rule, next) || next > to_ms {
            break;
        }
        if next >= from_ms {
            if remaining <= 0 {
                break;
            }
            out.push(next);
            remaining -= 1;
        }
        prev = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Todo;

    fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        at_time(
            NaiveDate::from_ymd_opt(y, mo, d).unwrap(),
            NaiveTime::from_hms_opt(h, mi, 0).unwrap(),
        )
        .unwrap()
    }

    fn rule(mode: &str) -> RepeatRule {
        RepeatRule {
            mode: mode.to_string(),
            every: None,
            unit: None,
            weekdays: None,
            month_day: None,
            month_nth: None,
            end_mode: None,
            end_at: None,
            count: None,
        }
    }

    fn todo_with(rule: RepeatRule, due: i64) -> Todo {
        Todo {
            id: 1,
            title: "t".into(),
            done: false,
            priority: 0,
            created_at: String::new(),
            updated_at: String::new(),
            completed_at: None,
            due_at: Some(due),
            remind_at: None,
            remind_fired: false,
            parent_id: None,
            sort_order: None,
            version: 0,
            description: String::new(),
            pinned: false,
            repeat_mode: rule.mode.clone(),
            repeat_every: rule.every,
            repeat_unit: rule.unit.clone(),
            repeat_weekdays: rule.weekdays,
            repeat_month_day: rule.month_day,
            repeat_month_nth: rule.month_nth,
            repeat_end_mode: rule.end_mode.clone(),
            repeat_end_at: rule.end_at,
            repeat_count: rule.count,
            repeat_done_count: 0,
            repeat_last_done_at: None,
        }
    }

    #[test]
    fn daily_keeps_time_of_day() {
        let due = ts(2026, 9, 21, 9, 30);
        let r = rule("daily");
        let next = next_occurrence(&r, due, due).unwrap();
        assert_eq!(next, ts(2026, 9, 22, 9, 30));
    }

    #[test]
    fn daily_skips_missed_periods() {
        // 逾期 3 天不补历史：直接滚到最近一个未来时刻
        let due = ts(2026, 9, 21, 9, 0);
        let now = ts(2026, 9, 24, 10, 0);
        let r = rule("daily");
        assert_eq!(next_occurrence(&r, due, now).unwrap(), ts(2026, 9, 25, 9, 0));
    }

    #[test]
    fn weekdays_skip_weekend() {
        // 2026-09-25 是周五 → 下一轮周一 09-28
        let due = ts(2026, 9, 25, 9, 0);
        let r = rule("weekdays");
        assert_eq!(next_occurrence(&r, due, due).unwrap(), ts(2026, 9, 28, 9, 0));
    }

    #[test]
    fn weekly_with_mask_picks_selected_days() {
        // 周一 + 周三（bit0 + bit2）
        let mut r = rule("weekly");
        r.weekdays = Some(0b0000101);
        let due = ts(2026, 9, 21, 8, 0); // 周一
        let next = next_occurrence(&r, due, due).unwrap();
        assert_eq!(next, ts(2026, 9, 23, 8, 0)); // 周三
        let next2 = next_occurrence(&r, next, next).unwrap();
        assert_eq!(next2, ts(2026, 9, 28, 8, 0)); // 下周一
    }

    #[test]
    fn monthly_clamps_to_month_end() {
        // 每月 31 号：2 月取 28 号（2027 非闰年）
        let mut r = rule("monthly");
        r.month_day = Some(31);
        let due = ts(2027, 1, 31, 9, 0);
        assert_eq!(next_occurrence(&r, due, due).unwrap(), ts(2027, 2, 28, 9, 0));
    }

    #[test]
    fn monthly_last_day_uses_actual_month_end() {
        let mut r = rule("monthly");
        r.month_day = Some(-1);
        let due = ts(2027, 1, 31, 9, 0);
        assert_eq!(next_occurrence(&r, due, due).unwrap(), ts(2027, 2, 28, 9, 0));
    }

    #[test]
    fn monthly_nth_weekday() {
        // 每月第 2 个星期一
        let mut r = rule("monthly");
        r.month_nth = Some(2);
        r.weekdays = Some(1 << 0);
        let due = ts(2026, 9, 14, 9, 0); // 9 月第 2 个周一
        assert_eq!(next_occurrence(&r, due, due).unwrap(), ts(2026, 10, 12, 9, 0));
    }

    #[test]
    fn yearly_keeps_month_day() {
        let r = rule("yearly");
        let due = ts(2026, 3, 15, 10, 0);
        assert_eq!(next_occurrence(&r, due, due).unwrap(), ts(2027, 3, 15, 10, 0));
    }

    #[test]
    fn custom_every_n_days() {
        let mut r = rule("custom");
        r.every = Some(3);
        r.unit = Some("day".into());
        let due = ts(2026, 9, 21, 9, 0);
        assert_eq!(next_occurrence(&r, due, due).unwrap(), ts(2026, 9, 24, 9, 0));
    }

    #[test]
    fn until_end_stops_expansion() {
        let mut r = rule("daily");
        r.end_mode = Some("until".into());
        r.end_at = Some(ts(2026, 9, 23, 23, 59));
        let due = ts(2026, 9, 21, 9, 0);
        assert_eq!(next_occurrence(&r, due, due).unwrap(), ts(2026, 9, 22, 9, 0));
        let beyond = ts(2026, 9, 23, 9, 0);
        assert_eq!(next_occurrence(&r, beyond, beyond), None);
    }

    #[test]
    fn count_end_truncates_expansion() {
        let mut r = rule("daily");
        r.end_mode = Some("count".into());
        r.count = Some(3);
        let due = ts(2026, 9, 21, 9, 0);
        // 已做 1 次 → 还剩 2 个未来实例
        let got = expand_occurrences(&r, due, 1, due, ts(2026, 10, 10, 0, 0));
        assert_eq!(got, vec![ts(2026, 9, 22, 9, 0), ts(2026, 9, 23, 9, 0)]);
    }

    #[test]
    fn expand_lists_virtual_occurrences_in_range() {
        let r = rule("daily");
        let due = ts(2026, 9, 21, 9, 0);
        let got = expand_occurrences(&r, due, 0, ts(2026, 9, 22, 0, 0), ts(2026, 9, 25, 23, 59));
        assert_eq!(
            got,
            vec![
                ts(2026, 9, 22, 9, 0),
                ts(2026, 9, 23, 9, 0),
                ts(2026, 9, 24, 9, 0),
                ts(2026, 9, 25, 9, 0),
            ]
        );
    }

    #[test]
    fn previous_occurrence_rolls_back() {
        let r = rule("daily");
        let cur = ts(2026, 9, 22, 9, 0);
        assert_eq!(previous_occurrence(&r, cur).unwrap(), ts(2026, 9, 21, 9, 0));
    }

    #[test]
    fn weekly_mask_previous_occurrence() {
        let mut r = rule("weekly");
        r.weekdays = Some(0b0000101); // 周一 + 周三
        let cur = ts(2026, 9, 23, 8, 0); // 周三
        assert_eq!(previous_occurrence(&r, cur).unwrap(), ts(2026, 9, 21, 8, 0)); // 周一
    }

    #[test]
    fn once_has_no_occurrences() {
        let r = rule("once");
        let due = ts(2026, 9, 21, 9, 0);
        assert!(next_occurrence(&r, due, due).is_none());
        assert!(expand_occurrences(&r, due, 0, due, due + 1).is_empty());
    }

    #[test]
    fn is_recurring_only_for_non_once() {
        assert!(!is_recurring(&todo_with(rule("once"), 0)));
        assert!(is_recurring(&todo_with(rule("daily"), 0)));
    }
}
