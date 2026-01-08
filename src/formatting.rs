// SPDX-License-Identifier: GPL-3.0-only

use crate::fl;
use cosmic::cosmic_config::ConfigGet;

/// Check if user prefers 24-hour time format from COSMIC settings
pub fn use_military_time() -> bool {
    cosmic::cosmic_config::Config::new("com.system76.CosmicAppletTime", 1)
        .ok()
        .and_then(|config| config.get::<bool>("military_time").ok())
        .unwrap_or(false)
}

/// Format a time according to user's COSMIC time preference
pub fn format_time(dt: &chrono::DateTime<chrono::Local>, include_day: bool) -> String {
    let time_fmt = if use_military_time() {
        "%H:%M"
    } else {
        "%I:%M %p"
    };
    if include_day {
        dt.format(&format!("%A, %B %d at {}", time_fmt)).to_string()
    } else {
        dt.format(&format!("%a {}", time_fmt)).to_string()
    }
}

/// Smart panel time formatting: just time if today, day+time if different day
pub fn format_panel_time(
    dt: &chrono::DateTime<chrono::Local>,
    now: &chrono::DateTime<chrono::Local>,
) -> String {
    let time_fmt = if use_military_time() {
        "%H:%M"
    } else {
        "%l:%M%P"
    }; // %l = hour 1-12 no padding, %P = lowercase am/pm
    let is_same_day = dt.date_naive() == now.date_naive();

    if is_same_day {
        // Just show time: "2:30pm" or "14:30"
        dt.format(time_fmt).to_string().trim().to_string()
    } else {
        // Show day and time: "Fri 2:30pm" or "Fri 14:30"
        dt.format(&format!("%a {}", time_fmt))
            .to_string()
            .trim()
            .to_string()
    }
}

/// Format a duration as relative time (e.g., "in 2d 3h" or "in 2h 30m")
/// Shows minutes only when the event is within 24 hours
pub fn format_relative_time(duration: chrono::Duration) -> String {
    let total_minutes = duration.num_minutes();
    if total_minutes < 0 {
        return fl!("time-now");
    }

    let days = total_minutes / (24 * 60);
    let hours = (total_minutes % (24 * 60)) / 60;
    let minutes = total_minutes % 60;

    if days > 0 {
        // More than a day away - show days and hours, skip minutes
        if hours > 0 {
            fl!("time-in-days-hours", days = days, hours = hours)
        } else {
            fl!("time-in-days", days = days)
        }
    } else if hours > 0 {
        // Within 24 hours - show hours and minutes
        if minutes > 0 {
            fl!("time-in-hours-minutes", hours = hours, minutes = minutes)
        } else {
            fl!("time-in-hours", hours = hours)
        }
    } else {
        fl!("time-in-minutes", minutes = minutes)
    }
}

/// Parse a hex color string (e.g., "#62a0ea") to an iced Color
pub fn parse_hex_color(hex: &str) -> Option<cosmic::iced::Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(cosmic::iced::Color::from_rgb8(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color_with_hash() {
        let color = parse_hex_color("#62a0ea").unwrap();
        assert_eq!(color.r, 0x62 as f32 / 255.0);
        assert_eq!(color.g, 0xa0 as f32 / 255.0);
        assert_eq!(color.b, 0xea as f32 / 255.0);
    }

    #[test]
    fn test_parse_hex_color_without_hash() {
        let color = parse_hex_color("ff0000").unwrap();
        assert_eq!(color.r, 1.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
    }

    #[test]
    fn test_parse_hex_color_invalid_length() {
        assert!(parse_hex_color("#fff").is_none());
        assert!(parse_hex_color("#fffffff").is_none());
        assert!(parse_hex_color("").is_none());
    }

    #[test]
    fn test_parse_hex_color_invalid_chars() {
        assert!(parse_hex_color("#gggggg").is_none());
        assert!(parse_hex_color("#zzzzzz").is_none());
    }
}
