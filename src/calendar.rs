// SPDX-License-Identifier: GPL-3.0-only

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use regex::Regex;
use zbus::{Connection, zvariant};
use ical::parser::ical::IcalParser;

/// User's attendance status for a meeting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttendanceStatus {
    /// User has accepted the meeting
    Accepted,
    /// User has tentatively accepted
    Tentative,
    /// User has declined
    Declined,
    /// User hasn't responded yet
    NeedsAction,
    /// No attendance info (user is organizer or it's a personal event)
    #[default]
    None,
}

#[derive(Debug, Clone)]
pub struct Meeting {
    pub uid: String,
    pub title: String,
    pub start: DateTime<Local>,
    #[allow(dead_code)]
    pub end: DateTime<Local>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub calendar_uid: String,
    pub is_all_day: bool,
    pub attendance_status: AttendanceStatus,
}

#[derive(Debug, Clone)]
pub struct CalendarInfo {
    pub uid: String,
    pub display_name: String,
    pub color: Option<String>,
}

/// Fetch available calendars from Evolution Data Server via D-Bus
pub async fn get_available_calendars() -> Vec<CalendarInfo> {
    // Debug: simulate no calendars for testing
    if std::env::var("DEBUG_NO_CALENDARS").is_ok() {
        return Vec::new();
    }

    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    get_calendars_from_dbus(&conn).await.unwrap_or_default()
}

/// Fetch upcoming meetings from Evolution Data Server via D-Bus
/// If enabled_uids is empty, all calendars are queried.
/// Otherwise, only calendars with UIDs in the list are queried.
/// Returns up to `limit` meetings (use limit=0 for just the next meeting info).
/// `additional_emails` are extra email addresses to identify the user in ATTENDEE fields.
pub async fn get_upcoming_meetings(enabled_uids: &[String], limit: usize, additional_emails: &[String]) -> Vec<Meeting> {
    // Debug: simulate no calendars for testing
    if std::env::var("DEBUG_NO_CALENDARS").is_ok() {
        return Vec::new();
    }

    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    get_meetings_from_dbus(&conn, enabled_uids, limit.max(1), additional_emails).await
}

async fn get_meetings_from_dbus(conn: &Connection, enabled_uids: &[String], limit: usize, additional_emails: &[String]) -> Vec<Meeting> {
    // Evolution Data Server workflow:
    // 1. Get calendar source UIDs from D-Bus SourceManager
    // 2. For each source, use CalendarFactory.OpenCalendar to get a calendar object
    // 3. Query the calendar object for events using GetObjectList
    // 4. Parse the iCalendar objects

    // Step 1: Get calendar source UIDs from D-Bus SourceManager
    let mut source_uids = match get_calendar_source_uids(conn).await {
        Some(uids) => uids,
        None => return Vec::new(),
    };

    // Filter to only enabled calendars if a filter is specified
    if !enabled_uids.is_empty() {
        source_uids.retain(|uid| enabled_uids.contains(uid));
    }

    // Step 2: Open calendars and get events
    let calendar_factory_proxy = match zbus::Proxy::new(
        conn,
        "org.gnome.evolution.dataserver.Calendar8",
        "/org/gnome/evolution/dataserver/CalendarFactory",
        "org.gnome.evolution.dataserver.CalendarFactory",
    )
    .await
    {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut all_meetings: Vec<Meeting> = Vec::new();
    let now = Local::now();

    for source_uid in source_uids {
        // Open the calendar for this source
        let (calendar_path, bus_name): (String, String) = match calendar_factory_proxy
            .call_method("OpenCalendar", &(source_uid.as_str(),))
            .await
        {
            Ok(reply) => match reply.body::<(String, String)>() {
                Ok((path, bus)) => (path, bus),
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        // Step 3: Query the calendar for events using GetObjectList
        let calendar_proxy = match zbus::Proxy::new(
            conn,
            bus_name.as_str(),
            calendar_path.as_str(),
            "org.gnome.evolution.dataserver.Calendar",
        )
        .await
        {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Get the CalEmailAddress property for this calendar
        // This is used to identify the user in ATTENDEE fields
        let cal_email: Option<String> = calendar_proxy
            .get_property::<String>("CalEmailAddress")
            .await
            .ok();

        // Combine CalEmailAddress with additional_emails for user identification
        // Filter out empty strings from additional_emails
        let mut user_emails: Vec<String> = additional_emails
            .iter()
            .filter(|e| !e.trim().is_empty())
            .cloned()
            .collect();
        if let Some(email) = cal_email {
            if !email.is_empty() && !user_emails.iter().any(|e| e.eq_ignore_ascii_case(&email)) {
                user_emails.push(email);
            }
        }

        // GetObjectList takes a query string - empty string gets all events
        // We could use ECalQuery format for filtering, but empty works for now
        let ics_objects: Vec<String> = match calendar_proxy
            .call_method("GetObjectList", &("",))
            .await
        {
            Ok(reply) => match reply.body::<Vec<String>>() {
                Ok(objects) => objects,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        // Step 4: Parse iCalendar objects and extract meetings
        for ics_object in ics_objects {
            // EDS returns raw VEVENT objects without VCALENDAR wrapper
            // The ical crate needs the wrapper, so add it if missing
            let wrapped = if ics_object.trim().starts_with("BEGIN:VEVENT") {
                format!("BEGIN:VCALENDAR\nVERSION:2.0\n{}\nEND:VCALENDAR", ics_object)
            } else {
                ics_object.clone()
            };

            if let Some(Ok(calendar)) = IcalParser::new(wrapped.as_bytes()).next() {
                for event in calendar.events {
                    // Check if this is an all-day event (DTSTART has VALUE=DATE)
                    let dtstart_prop = event.properties.iter().find(|p| p.name == "DTSTART");
                    let is_all_day = dtstart_prop.map_or(false, |p| {
                        // Check if VALUE=DATE is in the parameters or if the value is just a date (8 digits)
                        let has_date_param = p.params.as_ref().map_or(false, |params| {
                            params.iter().any(|(name, values)| {
                                name == "VALUE" && values.iter().any(|v| v == "DATE")
                            })
                        });
                        let value_is_date = p.value.as_ref().map_or(false, |v| {
                            let v = v.trim();
                            v.len() == 8 && v.chars().all(|c| c.is_ascii_digit())
                        });
                        has_date_param || value_is_date
                    });

                    // Get start and end times
                    let start_dt = dtstart_prop
                        .and_then(|p| p.value.as_ref())
                        .and_then(|v| parse_ical_datetime(v, &now));

                    let end_dt = event
                        .properties
                        .iter()
                        .find(|p| p.name == "DTEND" || p.name == "DURATION")
                        .and_then(|p| p.value.as_ref())
                        .and_then(|v| parse_ical_datetime(v, &now));

                    // Parse attendance status from ATTENDEE properties
                    let attendance_status = parse_attendance_status(&event.properties, &user_emails);

                    if let Some(start) = start_dt {
                        // Use end time if available, otherwise assume 1 hour duration
                        let end = end_dt.unwrap_or_else(|| start + chrono::Duration::hours(1));

                        // Only include future meetings
                        if start > now {
                            let uid = event
                                .properties
                                .iter()
                                .find(|p| p.name == "UID")
                                .and_then(|p| p.value.as_ref())
                                .map(|s| s.to_string())
                                .unwrap_or_default();

                            let title = event
                                .properties
                                .iter()
                                .find(|p| p.name == "SUMMARY")
                                .and_then(|p| p.value.as_ref())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "Untitled Event".to_string());

                            let location = event
                                .properties
                                .iter()
                                .find(|p| p.name == "LOCATION")
                                .and_then(|p| p.value.as_ref())
                                .map(|s| s.to_string());

                            let description = event
                                .properties
                                .iter()
                                .find(|p| p.name == "DESCRIPTION")
                                .and_then(|p| p.value.as_ref())
                                .map(|s| s.to_string());

                            all_meetings.push(Meeting {
                                uid,
                                title,
                                start,
                                end,
                                location,
                                description,
                                calendar_uid: source_uid.clone(),
                                is_all_day,
                                attendance_status,
                            });
                        }
                    }
                }
            }
        }
    }

    // Sort and return up to `limit` meetings
    all_meetings.sort_by_key(|m| m.start);
    all_meetings.into_iter().take(limit).collect()
}

/// Get calendar source UIDs from Evolution Data Server via D-Bus
///
/// This queries the SourceManager's ObjectManager interface to discover
/// all sources, including those from GNOME Online Accounts which are
/// not stored as files in ~/.config/evolution/sources/
async fn get_calendar_source_uids(conn: &Connection) -> Option<Vec<String>> {
    use std::collections::HashMap;
    use zvariant::{OwnedObjectPath, OwnedValue, Value};

    let source_manager_proxy = zbus::Proxy::new(
        conn,
        "org.gnome.evolution.dataserver.Sources5",
        "/org/gnome/evolution/dataserver/SourceManager",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    .ok()?;

    // GetManagedObjects returns a{oa{sa{sv}}} - dict of object paths to interface properties
    let reply = source_manager_proxy
        .call_method("GetManagedObjects", &())
        .await
        .ok()?;

    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        reply.body().ok()?;

    let mut source_uids = Vec::new();

    for (_path, interfaces) in objects {
        // Look for the Source interface
        if let Some(source_props) = interfaces.get("org.gnome.evolution.dataserver.Source") {
            // Get the UID
            let uid = source_props.get("UID").and_then(|v| {
                if let Some(Value::Str(s)) = v.downcast_ref::<Value>() {
                    Some(s.to_string())
                } else {
                    None
                }
            });

            // Get the Data (source configuration) and check for [Calendar] section
            let has_calendar = source_props.get("Data").map_or(false, |v| {
                if let Some(Value::Str(s)) = v.downcast_ref::<Value>() {
                    s.contains("[Calendar]")
                } else {
                    false
                }
            });

            if let Some(uid) = uid {
                if has_calendar {
                    source_uids.push(uid);
                }
            }
        }
    }

    Some(source_uids)
}

/// Get calendar info (UID and display name) from Evolution Data Server via D-Bus
async fn get_calendars_from_dbus(conn: &Connection) -> Option<Vec<CalendarInfo>> {
    use std::collections::HashMap;
    use zvariant::{OwnedObjectPath, OwnedValue, Value};

    let source_manager_proxy = zbus::Proxy::new(
        conn,
        "org.gnome.evolution.dataserver.Sources5",
        "/org/gnome/evolution/dataserver/SourceManager",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    .ok()?;

    let reply = source_manager_proxy
        .call_method("GetManagedObjects", &())
        .await
        .ok()?;

    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        reply.body().ok()?;

    let mut calendars = Vec::new();

    for (_path, interfaces) in objects {
        if let Some(source_props) = interfaces.get("org.gnome.evolution.dataserver.Source") {
            // Get the UID
            let uid = source_props.get("UID").and_then(|v| {
                if let Some(Value::Str(s)) = v.downcast_ref::<Value>() {
                    Some(s.to_string())
                } else {
                    None
                }
            });

            // Get the Data field and extract DisplayName and check for [Calendar]
            let data = source_props.get("Data").and_then(|v| {
                if let Some(Value::Str(s)) = v.downcast_ref::<Value>() {
                    Some(s.to_string())
                } else {
                    None
                }
            });

            if let (Some(uid), Some(data)) = (uid, data) {
                // Only include if it has a [Calendar] section
                if data.contains("[Calendar]") {
                    let display_name = parse_display_name(&data).unwrap_or_else(|| uid.clone());
                    let color = parse_color(&data);
                    calendars.push(CalendarInfo { uid, display_name, color });
                }
            }
        }
    }

    // Sort by display name for consistent ordering
    calendars.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Some(calendars)
}

/// Parse DisplayName from INI-format source data
fn parse_display_name(data: &str) -> Option<String> {
    // Look for DisplayName= line (without locale suffix like DisplayName[en])
    for line in data.lines() {
        let line = line.trim();
        if line.starts_with("DisplayName=") {
            return Some(line.strip_prefix("DisplayName=")?.to_string());
        }
    }
    None
}

/// Parse Color from INI-format source data (e.g., Color=#62a0ea)
fn parse_color(data: &str) -> Option<String> {
    for line in data.lines() {
        let line = line.trim();
        if line.starts_with("Color=") {
            return Some(line.strip_prefix("Color=")?.to_string());
        }
    }
    None
}

/// Parse attendance status from ATTENDEE properties
/// Matches the user's email addresses against ATTENDEE entries and extracts PARTSTAT
fn parse_attendance_status(properties: &[ical::property::Property], user_emails: &[String]) -> AttendanceStatus {
    // If no user emails provided, we can't determine attendance
    if user_emails.is_empty() {
        return AttendanceStatus::None;
    }

    // Normalize user emails to lowercase for comparison
    let user_emails_lower: Vec<String> = user_emails.iter().map(|e| e.to_lowercase()).collect();

    // Find all ATTENDEE properties
    for prop in properties.iter().filter(|p| p.name == "ATTENDEE") {
        let params = prop.params.as_ref();

        // Extract email from ATTENDEE - check EMAIL parameter first, then mailto: value
        let attendee_email = params
            .and_then(|params| {
                params.iter().find_map(|(name, values)| {
                    if name == "EMAIL" {
                        values.first().map(|v| v.to_lowercase())
                    } else {
                        None
                    }
                })
            })
            .or_else(|| {
                // Fall back to extracting from mailto: in the value
                prop.value.as_ref().and_then(|v| {
                    let v_lower = v.to_lowercase();
                    if v_lower.starts_with("mailto:") {
                        Some(v_lower.trim_start_matches("mailto:").to_string())
                    } else {
                        None
                    }
                })
            });

        // Check if this attendee matches any of the user's emails
        let is_user = attendee_email
            .as_ref()
            .map(|email| user_emails_lower.iter().any(|ue| ue == email))
            .unwrap_or(false);

        if is_user {
            // Extract PARTSTAT from parameters
            let partstat = params.and_then(|params| {
                params.iter().find_map(|(name, values)| {
                    if name == "PARTSTAT" {
                        values.first().map(|v| v.as_str())
                    } else {
                        None
                    }
                })
            });

            if let Some(status) = partstat {
                return match status.to_uppercase().as_str() {
                    "ACCEPTED" => AttendanceStatus::Accepted,
                    "TENTATIVE" => AttendanceStatus::Tentative,
                    "DECLINED" => AttendanceStatus::Declined,
                    "NEEDS-ACTION" => AttendanceStatus::NeedsAction,
                    _ => AttendanceStatus::None,
                };
            }
        }
    }

    // No matching ATTENDEE found - this is likely a personal event or user is organizer
    AttendanceStatus::None
}

fn parse_ical_datetime(value: &str, _default_tz: &DateTime<Local>) -> Option<DateTime<Local>> {
    // The value might be in formats like:
    // - "20240221T123000" (local time)
    // - "20240221T123000Z" (UTC)
    // - "TZID=America/Los_Angeles:20240221T123000" (with timezone param)
    // - "VALUE=DATE:20250527" (date only)

    // Extract the actual datetime value (after the last colon if present)
    let value = if value.contains(':') {
        value.split(':').last().unwrap_or(value)
    } else {
        value
    };
    let value = value.trim();

    // Try parsing as ISO 8601 format first
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Some(dt.with_timezone(&Local));
    }

    // Handle UTC times (ending with Z)
    if value.ends_with('Z') {
        let value = &value[..value.len()-1];
        if value.len() >= 15 {
            if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
                return Some(chrono::Utc.from_utc_datetime(&naive).with_timezone(&Local));
            }
        }
    }

    // Try parsing as YYYYMMDDTHHMMSS format (local time)
    if value.len() >= 15 && value.contains('T') {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
            return Local.from_local_datetime(&naive).single();
        }
    }

    // Try parsing as date only (YYYYMMDD)
    if value.len() == 8 && value.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(naive) = NaiveDateTime::parse_from_str(
            &format!("{}T000000", value),
            "%Y%m%dT%H%M%S"
        ) {
            return Local.from_local_datetime(&naive).single();
        }
    }

    None
}

/// Extract a meeting URL from the meeting's location or description fields
/// Checks location first (most common place for meeting links), then description
pub fn extract_meeting_url(meeting: &Meeting, patterns: &[String]) -> Option<String> {
    // Compile patterns, skipping any invalid ones
    let compiled: Vec<Regex> = patterns
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

    if compiled.is_empty() {
        return None;
    }

    // Check location first
    if let Some(ref location) = meeting.location {
        for regex in &compiled {
            if let Some(m) = regex.find(location) {
                return Some(m.as_str().to_string());
            }
        }
    }

    // Then check description
    if let Some(ref description) = meeting.description {
        for regex in &compiled {
            if let Some(m) = regex.find(description) {
                return Some(m.as_str().to_string());
            }
        }
    }

    None
}

/// Get the physical location from a meeting (location that is not a URL)
/// Returns None if the location is empty or appears to be just a URL
pub fn get_physical_location(meeting: &Meeting, url_patterns: &[String]) -> Option<String> {
    let location = meeting.location.as_ref()?;
    let location = location.trim();

    if location.is_empty() {
        return None;
    }

    // Compile patterns to check if location is a URL
    let compiled: Vec<Regex> = url_patterns
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

    // If location matches any URL pattern entirely, it's not a physical location
    for regex in &compiled {
        if let Some(m) = regex.find(location) {
            // If the match covers the entire location, skip it
            if m.start() == 0 && m.end() == location.len() {
                return None;
            }
        }
    }

    // Also skip if it looks like a generic URL
    if location.starts_with("http://") || location.starts_with("https://") {
        return None;
    }

    Some(location.to_string())
}
