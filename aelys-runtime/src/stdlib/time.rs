//! std.time

use crate::stdlib::helpers::{get_handle, get_int, get_string, make_string};
use crate::stdlib::{Resource, StdModuleExports, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut all_exports = Vec::new();
    let mut native_functions = Vec::new();

    macro_rules! reg_fn {
        ($name:expr, $arity:expr, $func:expr) => {{
            register_native(vm, "time", $name, $arity, $func)?;
            all_exports.push($name.to_string());
            native_functions.push(format!("time::{}", $name));
        }};
    }

    reg_fn!("now", 0, native_now);
    reg_fn!("now_ms", 0, native_now_ms);
    reg_fn!("now_us", 0, native_now_us);
    reg_fn!("now_ns", 0, native_now_ns);
    reg_fn!("timer", 0, native_timer);
    reg_fn!("elapsed", 1, native_elapsed);
    reg_fn!("elapsed_ms", 1, native_elapsed_ms);
    reg_fn!("elapsed_us", 1, native_elapsed_us);
    reg_fn!("reset", 1, native_reset);
    reg_fn!("sleep", 1, native_sleep);
    reg_fn!("sleep_us", 1, native_sleep_us);
    reg_fn!("year", 0, native_year);
    reg_fn!("month", 0, native_month);
    reg_fn!("day", 0, native_day);
    reg_fn!("hour", 0, native_hour);
    reg_fn!("minute", 0, native_minute);
    reg_fn!("second", 0, native_second);
    reg_fn!("weekday", 0, native_weekday);
    reg_fn!("yearday", 0, native_yearday);
    reg_fn!("format", 1, native_format);
    reg_fn!("iso", 0, native_iso);
    reg_fn!("date", 0, native_date);
    reg_fn!("time_str", 0, native_time_str);

    Ok(StdModuleExports {
        all_exports,
        native_functions,
    })
}

fn time_error(vm: &VM, op: &'static str, msg: String) -> RuntimeError {
    vm.runtime_error(RuntimeErrorKind::TypeError {
        operation: op,
        expected: "valid time operation",
        got: msg,
    })
}

fn native_now(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() as f64 + duration.subsec_nanos() as f64 / 1_000_000_000.0;
    Ok(Value::float(secs))
}

fn native_now_ms(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(Value::int(duration.as_millis() as i64))
}
fn native_now_us(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(Value::int(duration.as_micros() as i64))
}

/// now_ns() - Get current timestamp in nanoseconds since Unix epoch.
fn native_now_ns(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    // Note: This may overflow for dates far in the future
    Ok(Value::int(duration.as_nanos() as i64))
}

/// timer() - Create a new timer.
/// Returns a handle that can be used with elapsed().
fn native_timer(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = vm.store_resource(Resource::Timer(Instant::now()));
    Ok(Value::int(handle as i64))
}

/// elapsed(handle) - Get elapsed time in seconds since timer creation.
fn native_elapsed(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "time.elapsed")?;

    if let Some(Resource::Timer(instant)) = vm.get_resource(handle) {
        let elapsed = instant.elapsed();
        let secs = elapsed.as_secs() as f64 + elapsed.subsec_nanos() as f64 / 1_000_000_000.0;
        Ok(Value::float(secs))
    } else {
        Err(time_error(
            vm,
            "time.elapsed",
            "invalid timer handle".to_string(),
        ))
    }
}

/// elapsed_ms(handle) - Get elapsed time in milliseconds.
fn native_elapsed_ms(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "time.elapsed_ms")?;

    if let Some(Resource::Timer(instant)) = vm.get_resource(handle) {
        Ok(Value::int(instant.elapsed().as_millis() as i64))
    } else {
        Err(time_error(
            vm,
            "time.elapsed_ms",
            "invalid timer handle".to_string(),
        ))
    }
}

/// elapsed_us(handle) - Get elapsed time in microseconds.
fn native_elapsed_us(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "time.elapsed_us")?;

    if let Some(Resource::Timer(instant)) = vm.get_resource(handle) {
        Ok(Value::int(instant.elapsed().as_micros() as i64))
    } else {
        Err(time_error(
            vm,
            "time.elapsed_us",
            "invalid timer handle".to_string(),
        ))
    }
}

/// reset(handle) - Reset timer to current time.
fn native_reset(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "time.reset")?;

    if let Some(Resource::Timer(instant)) = vm.get_resource_mut(handle) {
        *instant = Instant::now();
        Ok(Value::null())
    } else {
        Err(time_error(
            vm,
            "time.reset",
            "invalid timer handle".to_string(),
        ))
    }
}

/// sleep(ms) - Sleep for given milliseconds.
fn native_sleep(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let ms = get_int(vm, args[0], "time.sleep")?;
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms as u64));
    }
    Ok(Value::null())
}

/// sleep_us(us) - Sleep for given microseconds.
fn native_sleep_us(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let us = get_int(vm, args[0], "time.sleep_us")?;
    if us > 0 {
        std::thread::sleep(Duration::from_micros(us as u64));
    }
    Ok(Value::null())
}

/// Get the current local time broken down into components.
fn get_local_time() -> (i32, u32, u32, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Simple UTC->local conversion is complex, so we'll use UTC for now
    // A real implementation would use the system's local timezone

    // Calculate date components from Unix timestamp
    let days = secs / 86400;
    let time_of_day = secs % 86400;

    let hour = (time_of_day / 3600) as u32;
    let minute = ((time_of_day % 3600) / 60) as u32;
    let second = (time_of_day % 60) as u32;

    // Calculate year, month, day from days since epoch (1970-01-01)
    let (year, month, day, yearday) = days_to_ymd(days as i32);

    // Calculate weekday (1970-01-01 was Thursday = 4)
    let weekday = ((days % 7 + 4) % 7) as u32;

    (year, month, day, hour, minute, second, weekday, yearday)
}

/// Convert days since epoch to (year, month, day, yearday).
fn days_to_ymd(mut days: i32) -> (i32, u32, u32, u32) {
    // Algorithm from Howard Hinnant
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };

    // Calculate day of year
    let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_before_month = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut yearday = days_before_month[m as usize - 1] + d;
    if is_leap && m > 2 {
        yearday += 1;
    }

    (year, m, d, yearday)
}

/// year() - Get current year.
fn native_year(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (year, _, _, _, _, _, _, _) = get_local_time();
    Ok(Value::int(year as i64))
}

/// month() - Get current month (1-12).
fn native_month(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (_, month, _, _, _, _, _, _) = get_local_time();
    Ok(Value::int(month as i64))
}

/// day() - Get current day of month (1-31).
fn native_day(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (_, _, day, _, _, _, _, _) = get_local_time();
    Ok(Value::int(day as i64))
}

/// hour() - Get current hour (0-23).
fn native_hour(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (_, _, _, hour, _, _, _, _) = get_local_time();
    Ok(Value::int(hour as i64))
}

/// minute() - Get current minute (0-59).
fn native_minute(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (_, _, _, _, minute, _, _, _) = get_local_time();
    Ok(Value::int(minute as i64))
}

/// second() - Get current second (0-59).
fn native_second(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (_, _, _, _, _, second, _, _) = get_local_time();
    Ok(Value::int(second as i64))
}

/// weekday() - Get day of week (0=Sunday, 6=Saturday).
fn native_weekday(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (_, _, _, _, _, _, weekday, _) = get_local_time();
    Ok(Value::int(weekday as i64))
}

/// yearday() - Get day of year (1-366).
fn native_yearday(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (_, _, _, _, _, _, _, yearday) = get_local_time();
    Ok(Value::int(yearday as i64))
}

/// format(format_str) - Format current time.
/// Supports: %Y (year), %m (month), %d (day), %H (hour), %M (minute), %S (second)
fn native_format(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let format = get_string(vm, args[0], "time.format")?;
    let (year, month, day, hour, minute, second, weekday, yearday) = get_local_time();

    let weekday_names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let month_names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let result: String = format
        .replace("%Y", &format!("{:04}", year))
        .replace("%y", &format!("{:02}", year % 100))
        .replace("%m", &format!("{:02}", month))
        .replace("%d", &format!("{:02}", day))
        .replace("%H", &format!("{:02}", hour))
        .replace("%M", &format!("{:02}", minute))
        .replace("%S", &format!("{:02}", second))
        .replace("%j", &format!("{:03}", yearday))
        .replace("%w", &format!("{}", weekday))
        .replace("%a", weekday_names[weekday as usize])
        .replace("%b", month_names[(month - 1) as usize])
        .replace("%%", "%");

    Ok(make_string(vm, &result)?)
}

/// iso() - Get current time in ISO 8601 format.
fn native_iso(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (year, month, day, hour, minute, second, _, _) = get_local_time();
    let s = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    );
    Ok(make_string(vm, &s)?)
}

/// date() - Get current date as "YYYY-MM-DD".
fn native_date(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (year, month, day, _, _, _, _, _) = get_local_time();
    let s = format!("{:04}-{:02}-{:02}", year, month, day);
    Ok(make_string(vm, &s)?)
}

/// time_str() - Get current time as "HH:MM:SS".
fn native_time_str(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let (_, _, _, hour, minute, second, _, _) = get_local_time();
    let s = format!("{:02}:{:02}:{:02}", hour, minute, second);
    Ok(make_string(vm, &s)?)
}
