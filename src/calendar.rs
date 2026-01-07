// SPDX-License-Identifier: MPL-2.0

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use zbus::{Connection, zvariant};
use ical::parser::ical::IcalParser;

#[derive(Debug, Clone)]
pub struct Meeting {
    pub title: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub location: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CalendarInfo {
    pub uid: String,
    pub display_name: String,
    pub color: Option<String>,
}

/// Fetch available calendars from Evolution Data Server via D-Bus
pub async fn get_available_calendars() -> Vec<CalendarInfo> {
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
pub async fn get_upcoming_meetings(enabled_uids: &[String], limit: usize) -> Vec<Meeting> {
    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    get_meetings_from_dbus(&conn, enabled_uids, limit.max(1)).await
}

async fn get_meetings_from_dbus(conn: &Connection, enabled_uids: &[String], limit: usize) -> Vec<Meeting> {
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
                    // Get start and end times
                    let start_dt = event
                        .properties
                        .iter()
                        .find(|p| p.name == "DTSTART")
                        .and_then(|p| p.value.as_ref())
                        .and_then(|v| parse_ical_datetime(v, &now));
                    
                    let end_dt = event
                        .properties
                        .iter()
                        .find(|p| p.name == "DTEND" || p.name == "DURATION")
                        .and_then(|p| p.value.as_ref())
                        .and_then(|v| parse_ical_datetime(v, &now));
                    
                    if let Some(start) = start_dt {
                        // Use end time if available, otherwise assume 1 hour duration
                        let end = end_dt.unwrap_or_else(|| start + chrono::Duration::hours(1));

                        // Only include future meetings
                        if start > now {
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
                                title,
                                start,
                                end,
                                location,
                                description,
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
