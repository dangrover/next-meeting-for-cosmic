// SPDX-License-Identifier: GPL-3.0-only

use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone};
use futures_util::StreamExt;
use ical::parser::ical::IcalParser;
use regex::Regex;
use rrule::{RRuleSet, Tz};
use zbus::{Connection, zvariant};

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
    /// Last update timestamp from EDS (ISO 8601 format)
    pub last_synced: Option<String>,
    /// Backend type (e.g., "local", "caldav", "google")
    pub backend: Option<String>,
}

impl CalendarInfo {
    /// Returns true if this calendar is a valid source of meetings.
    /// Some calendars (contacts, weather, birthdays) don't contain actual meetings.
    #[must_use]
    pub fn is_meeting_source(&self) -> bool {
        match self.backend.as_deref() {
            Some("contacts") | Some("weather") | Some("birthdays") => false,
            _ => true,
        }
    }
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

/// Refresh all calendars by triggering an upstream sync with remote servers.
/// This calls the Refresh D-Bus method on each calendar, which forces EDS to
/// fetch the latest data from CalDAV/Google/etc servers.
/// If enabled_uids is empty, all calendars are refreshed.
pub async fn refresh_calendars(enabled_uids: &[String]) {
    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(_) => return,
    };

    // Get calendar source UIDs
    let mut source_uids = match get_calendar_source_uids(&conn).await {
        Some(uids) => uids,
        None => return,
    };

    // Filter to only enabled calendars if specified
    if !enabled_uids.is_empty() {
        source_uids.retain(|uid| enabled_uids.contains(uid));
    }

    // Open calendar factory
    let calendar_factory_proxy = match zbus::Proxy::new(
        &conn,
        "org.gnome.evolution.dataserver.Calendar8",
        "/org/gnome/evolution/dataserver/CalendarFactory",
        "org.gnome.evolution.dataserver.CalendarFactory",
    )
    .await
    {
        Ok(p) => p,
        Err(_) => return,
    };

    // Refresh each calendar
    for source_uid in source_uids {
        // Open the calendar
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

        // Get a proxy to the calendar
        let calendar_proxy = match zbus::Proxy::new(
            &conn,
            bus_name.as_str(),
            calendar_path.as_str(),
            "org.gnome.evolution.dataserver.Calendar",
        )
        .await
        {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Call Refresh method (fire and forget - don't wait for completion)
        let _ = calendar_proxy.call_method("Refresh", &()).await;
    }
}

/// Watch for calendar changes via D-Bus `PropertiesChanged` signals.
/// Returns a stream that yields () whenever any calendar's properties change.
/// This allows detecting when EDS has updated calendar data after a sync.
pub async fn watch_calendar_changes(
    enabled_uids: Vec<String>,
    sender: tokio::sync::mpsc::Sender<()>,
) {
    let Ok(conn) = Connection::session().await else {
        return;
    };

    // Get calendar source UIDs
    let Some(mut source_uids) = get_calendar_source_uids(&conn).await else {
        return;
    };

    // Filter to enabled calendars if specified
    if !enabled_uids.is_empty() {
        source_uids.retain(|uid| enabled_uids.contains(uid));
    }

    // Open calendar factory
    let Ok(calendar_factory_proxy) = zbus::Proxy::new(
        &conn,
        "org.gnome.evolution.dataserver.Calendar8",
        "/org/gnome/evolution/dataserver/CalendarFactory",
        "org.gnome.evolution.dataserver.CalendarFactory",
    )
    .await
    else {
        return;
    };

    // First collect all calendar (path, bus) pairs
    let mut calendar_info: Vec<(String, String)> = Vec::new();
    for source_uid in &source_uids {
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
        calendar_info.push((calendar_path, bus_name));
    }

    if calendar_info.is_empty() {
        return;
    }

    // Spawn a watcher task for each calendar
    // Each task watches for PropertiesChanged and sends to the shared channel
    let mut handles = Vec::new();
    for (calendar_path, bus_name) in calendar_info {
        let sender_clone = sender.clone();
        let conn_clone = conn.clone();
        handles.push(tokio::spawn(async move {
            watch_single_calendar(conn_clone, bus_name, calendar_path, sender_clone).await;
        }));
    }

    // Wait for all watcher tasks (they run indefinitely until cancelled)
    for handle in handles {
        let _ = handle.await;
    }
}

/// Watch a single calendar for `PropertiesChanged` signals
async fn watch_single_calendar(
    conn: Connection,
    bus_name: String,
    calendar_path: String,
    sender: tokio::sync::mpsc::Sender<()>,
) {
    // Create a proxy for the Properties interface on this calendar
    let Ok(props_proxy) = zbus::Proxy::new(
        &conn,
        bus_name.as_str(),
        calendar_path.as_str(),
        "org.freedesktop.DBus.Properties",
    )
    .await
    else {
        return;
    };

    // Subscribe to PropertiesChanged signals
    let Ok(mut stream) = props_proxy.receive_signal("PropertiesChanged").await else {
        return;
    };

    // Listen for signals and notify the channel
    while stream.next().await.is_some() {
        let _ = sender.try_send(());
    }
}

/// Fetch upcoming meetings from Evolution Data Server via D-Bus
/// If enabled_uids is empty, all calendars are queried.
/// Otherwise, only calendars with UIDs in the list are queried.
/// Returns up to `limit` meetings (use limit=0 for just the next meeting info).
/// `additional_emails` are extra email addresses to identify the user in ATTENDEE fields.
pub async fn get_upcoming_meetings(
    enabled_uids: &[String],
    limit: usize,
    additional_emails: &[String],
) -> Vec<Meeting> {
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

async fn get_meetings_from_dbus(
    conn: &Connection,
    enabled_uids: &[String],
    limit: usize,
    additional_emails: &[String],
) -> Vec<Meeting> {
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
        if let Some(email) = cal_email
            && !email.is_empty()
            && !user_emails.iter().any(|e| e.eq_ignore_ascii_case(&email))
        {
            user_emails.push(email);
        }

        // GetObjectList takes an S-expression query string
        // Use occur-in-time-range? to expand recurring events into instances
        // Query from now to 30 days in the future
        let now = Local::now();
        let end = now + chrono::Duration::days(30);
        // Convert to UTC for the query (EDS expects UTC timestamps)
        let now_utc = now.with_timezone(&chrono::Utc);
        let end_utc = end.with_timezone(&chrono::Utc);
        let query = format!(
            "(occur-in-time-range? (make-time \"{}\") (make-time \"{}\"))",
            now_utc.format("%Y%m%dT%H%M%SZ"),
            end_utc.format("%Y%m%dT%H%M%SZ")
        );

        let ics_objects: Vec<String> = match calendar_proxy
            .call_method("GetObjectList", &(query.as_str(),))
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
                format!(
                    "BEGIN:VCALENDAR\nVERSION:2.0\n{}\nEND:VCALENDAR",
                    ics_object
                )
            } else {
                ics_object.clone()
            };

            if let Some(Ok(calendar)) = IcalParser::new(wrapped.as_bytes()).next() {
                for event in calendar.events {
                    // Check if this is an all-day event (DTSTART has VALUE=DATE)
                    let dtstart_prop = event.properties.iter().find(|p| p.name == "DTSTART");
                    let is_all_day = dtstart_prop.is_some_and(|p| {
                        // Check if VALUE=DATE is in the parameters or if the value is just a date (8 digits)
                        let has_date_param = p.params.as_ref().is_some_and(|params| {
                            params.iter().any(|(name, values)| {
                                name == "VALUE" && values.iter().any(|v| v == "DATE")
                            })
                        });
                        let value_is_date = p.value.as_ref().is_some_and(|v| {
                            let v = v.trim();
                            v.len() == 8 && v.chars().all(|c| c.is_ascii_digit())
                        });
                        has_date_param || value_is_date
                    });

                    // Check for RRULE (recurring event)
                    let rrule_prop = event.properties.iter().find(|p| p.name == "RRULE");

                    // Check for RECURRENCE-ID (this is a modified instance, not the master)
                    let recurrence_id = event.properties.iter().find(|p| p.name == "RECURRENCE-ID");

                    // Parse attendance status from ATTENDEE properties
                    let attendance_status =
                        parse_attendance_status(&event.properties, &user_emails);

                    // Extract common meeting properties
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

                    // If this is a modified instance (has RECURRENCE-ID), use it directly
                    // The DTSTART in a modified instance is the actual occurrence time
                    if recurrence_id.is_some() {
                        let start_dt = dtstart_prop
                            .and_then(|p| p.value.as_ref())
                            .and_then(|v| parse_ical_datetime(v, &now));

                        let end_dt = event
                            .properties
                            .iter()
                            .find(|p| p.name == "DTEND")
                            .and_then(|p| p.value.as_ref())
                            .and_then(|v| parse_ical_datetime(v, &now));

                        if let Some(start) = start_dt {
                            let end = end_dt.unwrap_or_else(|| start + chrono::Duration::hours(1));

                            if start > now && start < end {
                                all_meetings.push(Meeting {
                                    uid: uid.clone(),
                                    title: title.clone(),
                                    start,
                                    end,
                                    location: location.clone(),
                                    description: description.clone(),
                                    calendar_uid: source_uid.clone(),
                                    is_all_day,
                                    attendance_status,
                                });
                            }
                        }
                        continue;
                    }

                    // If this is a recurring event (has RRULE), expand it
                    if let Some(rrule_val) = rrule_prop.and_then(|p| p.value.as_ref()) {
                        // Get DTSTART with timezone info for rrule
                        let dtstart_with_tz = dtstart_prop.map(|p| {
                            let tz = extract_timezone_from_prop(p);
                            let value = p.value.as_deref().unwrap_or_default();
                            (value, tz)
                        });

                        // Get duration from DTEND or DURATION
                        let duration = {
                            let start_dt = dtstart_prop
                                .and_then(|p| p.value.as_ref())
                                .and_then(|v| parse_ical_datetime(v, &now));
                            let end_dt = event
                                .properties
                                .iter()
                                .find(|p| p.name == "DTEND")
                                .and_then(|p| p.value.as_ref())
                                .and_then(|v| parse_ical_datetime(v, &now));

                            match (start_dt, end_dt) {
                                (Some(s), Some(e)) => e.signed_duration_since(s),
                                _ => Duration::hours(1),
                            }
                        };

                        // Collect EXDATE values
                        let exdates: Vec<&str> = event
                            .properties
                            .iter()
                            .filter(|p| p.name == "EXDATE")
                            .filter_map(|p| p.value.as_deref())
                            .collect();

                        // Expand the RRULE
                        if let Some((dtstart_val, tz)) = dtstart_with_tz {
                            let occurrences = expand_rrule(
                                dtstart_val,
                                rrule_val,
                                &exdates,
                                tz.as_deref(),
                                now,
                                end,
                            );

                            for occurrence_start in occurrences {
                                let occurrence_end = occurrence_start + duration;

                                all_meetings.push(Meeting {
                                    uid: format!(
                                        "{}@{}",
                                        uid,
                                        occurrence_start.format("%Y%m%dT%H%M%S")
                                    ),
                                    title: title.clone(),
                                    start: occurrence_start,
                                    end: occurrence_end,
                                    location: location.clone(),
                                    description: description.clone(),
                                    calendar_uid: source_uid.clone(),
                                    is_all_day,
                                    attendance_status,
                                });
                            }
                        }
                        continue;
                    }

                    // Non-recurring event: use DTSTART directly
                    let start_dt = dtstart_prop
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
                        let meeting_end =
                            end_dt.unwrap_or_else(|| start + chrono::Duration::hours(1));

                        // Only include future meetings
                        if start > now {
                            all_meetings.push(Meeting {
                                uid,
                                title,
                                start,
                                end: meeting_end,
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
            let has_calendar = source_props.get("Data").is_some_and(|v| {
                if let Some(Value::Str(s)) = v.downcast_ref::<Value>() {
                    s.contains("[Calendar]")
                } else {
                    false
                }
            });

            if let Some(uid) = uid
                && has_calendar
            {
                source_uids.push(uid);
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

            if let (Some(uid), Some(data)) = (uid, data)
                && data.contains("[Calendar]")
            {
                let display_name = parse_display_name(&data).unwrap_or_else(|| uid.clone());
                let color = parse_color(&data);
                let backend = parse_backend_name(&data);
                calendars.push(CalendarInfo {
                    uid,
                    display_name,
                    color,
                    last_synced: None, // Will be filled in below
                    backend,
                });
            }
        }
    }

    // Fetch last_synced (Revision) for each calendar
    if let Ok(factory_proxy) = zbus::Proxy::new(
        conn,
        "org.gnome.evolution.dataserver.Calendar8",
        "/org/gnome/evolution/dataserver/CalendarFactory",
        "org.gnome.evolution.dataserver.CalendarFactory",
    )
    .await
    {
        for cal in &mut calendars {
            if let Ok(reply) = factory_proxy
                .call_method("OpenCalendar", &(cal.uid.as_str(),))
                .await
            {
                if let Ok((calendar_path, bus_name)) = reply.body::<(String, String)>() {
                    if let Ok(cal_proxy) = zbus::Proxy::new(
                        conn,
                        bus_name.as_str(),
                        calendar_path.as_str(),
                        "org.gnome.evolution.dataserver.Calendar",
                    )
                    .await
                    {
                        // Get the Revision property (format: "2026-01-08T04:19:20Z(0)")
                        if let Ok(revision) = cal_proxy.get_property::<String>("Revision").await {
                            // Extract just the timestamp part before the parentheses
                            let timestamp = revision.split('(').next().unwrap_or(&revision);
                            cal.last_synced = Some(timestamp.to_string());
                        }
                    }
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

/// Parse BackendName from INI-format source data (in [Calendar] section)
fn parse_backend_name(data: &str) -> Option<String> {
    // Look for BackendName= in the [Calendar] section
    let mut in_calendar_section = false;
    for line in data.lines() {
        let line = line.trim();
        if line == "[Calendar]" {
            in_calendar_section = true;
        } else if line.starts_with('[') {
            in_calendar_section = false;
        } else if in_calendar_section && line.starts_with("BackendName=") {
            return Some(line.strip_prefix("BackendName=")?.to_string());
        }
    }
    None
}

/// Parse attendance status from ATTENDEE properties
/// Matches the user's email addresses against ATTENDEE entries and extracts PARTSTAT
fn parse_attendance_status(
    properties: &[ical::property::Property],
    user_emails: &[String],
) -> AttendanceStatus {
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
        value.split(':').next_back().unwrap_or(value)
    } else {
        value
    };
    let value = value.trim();

    // Try parsing as ISO 8601 format first
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Some(dt.with_timezone(&Local));
    }

    // Handle UTC times (ending with Z)
    if let Some(value) = value.strip_suffix('Z')
        && value.len() >= 15
        && let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
    {
        return Some(chrono::Utc.from_utc_datetime(&naive).with_timezone(&Local));
    }

    // Try parsing as YYYYMMDDTHHMMSS format (local time)
    if value.len() >= 15
        && value.contains('T')
        && let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
    {
        return Local.from_local_datetime(&naive).single();
    }

    // Try parsing as date only (YYYYMMDD)
    if value.len() == 8
        && value.chars().all(|c| c.is_ascii_digit())
        && let Ok(naive) =
            NaiveDateTime::parse_from_str(&format!("{}T000000", value), "%Y%m%dT%H%M%S")
    {
        return Local.from_local_datetime(&naive).single();
    }

    None
}

/// Extract timezone from an iCal property's TZID parameter
fn extract_timezone_from_prop(prop: &ical::property::Property) -> Option<String> {
    prop.params.as_ref().and_then(|params| {
        params.iter().find_map(|(name, values)| {
            if name == "TZID" {
                values.first().cloned()
            } else {
                None
            }
        })
    })
}

/// Parse an iCal timezone string to rrule Tz timezone
fn parse_ical_timezone(tz_str: &str) -> Option<Tz> {
    // Common IANA timezones (convert slashes to double underscores for rrule)
    // The rrule crate uses constants like Tz::America__Los_Angeles
    match tz_str {
        // US timezones
        "America/Los_Angeles" => Some(Tz::America__Los_Angeles),
        "America/New_York" => Some(Tz::America__New_York),
        "America/Chicago" => Some(Tz::America__Chicago),
        "America/Denver" => Some(Tz::America__Denver),
        "America/Phoenix" => Some(Tz::America__Phoenix),
        "America/Detroit" => Some(Tz::America__Detroit),
        "America/Indiana/Indianapolis" => Some(Tz::America__Indiana__Indianapolis),
        "America/Anchorage" => Some(Tz::America__Anchorage),
        // European timezones
        "Europe/London" => Some(Tz::Europe__London),
        "Europe/Paris" => Some(Tz::Europe__Paris),
        "Europe/Berlin" => Some(Tz::Europe__Berlin),
        "Europe/Amsterdam" => Some(Tz::Europe__Amsterdam),
        "Europe/Rome" => Some(Tz::Europe__Rome),
        "Europe/Madrid" => Some(Tz::Europe__Madrid),
        // Asian timezones
        "Asia/Tokyo" => Some(Tz::Asia__Tokyo),
        "Asia/Shanghai" => Some(Tz::Asia__Shanghai),
        "Asia/Singapore" => Some(Tz::Asia__Singapore),
        "Asia/Hong_Kong" => Some(Tz::Asia__Hong_Kong),
        "Asia/Kolkata" => Some(Tz::Asia__Kolkata),
        "Asia/Dubai" => Some(Tz::Asia__Dubai),
        // Pacific timezones
        "Pacific/Honolulu" => Some(Tz::Pacific__Honolulu),
        "Pacific/Auckland" => Some(Tz::Pacific__Auckland),
        "Australia/Sydney" => Some(Tz::Australia__Sydney),
        "Australia/Melbourne" => Some(Tz::Australia__Melbourne),
        // UTC
        "UTC" | "Etc/UTC" => Some(Tz::UTC),
        // Windows timezone aliases
        "Pacific Standard Time" | "Pacific Daylight Time" => Some(Tz::America__Los_Angeles),
        "Eastern Standard Time" | "Eastern Daylight Time" => Some(Tz::America__New_York),
        "Central Standard Time" | "Central Daylight Time" => Some(Tz::America__Chicago),
        "Mountain Standard Time" | "Mountain Daylight Time" => Some(Tz::America__Denver),
        _ => None,
    }
}

/// Expand a recurring event (RRULE) into individual occurrences within a time range
fn expand_rrule(
    dtstart_val: &str,
    rrule_val: &str,
    exdates: &[&str],
    tz_str: Option<&str>,
    range_start: DateTime<Local>,
    range_end: DateTime<Local>,
) -> Vec<DateTime<Local>> {
    // Parse the timezone
    let tz = tz_str.and_then(parse_ical_timezone).unwrap_or(Tz::UTC);

    // Parse the DTSTART value (just the time part, e.g., "20251211T080000")
    let dtstart_str = if dtstart_val.contains(':') {
        dtstart_val.split(':').next_back().unwrap_or(dtstart_val)
    } else {
        dtstart_val
    };

    // Parse naive datetime
    let naive_dt = if dtstart_str.len() == 8 && dtstart_str.chars().all(|c| c.is_ascii_digit()) {
        // Date only
        NaiveDateTime::parse_from_str(&format!("{}T000000", dtstart_str), "%Y%m%dT%H%M%S").ok()
    } else {
        NaiveDateTime::parse_from_str(dtstart_str.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()
    };

    let Some(naive_dt) = naive_dt else {
        return Vec::new();
    };

    // Build the DTSTART string for rrule crate
    // Format: DTSTART;TZID=America/Los_Angeles:20251211T080000
    let dtstart_for_rrule = if tz == Tz::UTC || dtstart_str.ends_with('Z') {
        format!("DTSTART:{}Z", naive_dt.format("%Y%m%dT%H%M%S"))
    } else {
        format!("DTSTART;TZID={}:{}", tz, naive_dt.format("%Y%m%dT%H%M%S"))
    };

    // Build the full RRuleSet string
    let mut rrule_str = format!("{}\nRRULE:{}", dtstart_for_rrule, rrule_val);

    // Add EXDATE entries
    for exdate in exdates {
        // Parse exdate value - extract the datetime part
        let exdate_val = if exdate.contains(':') {
            exdate.split(':').next_back().unwrap_or(exdate)
        } else {
            exdate
        };

        // Try to get timezone from EXDATE if present
        let exdate_tz = if exdate.contains("TZID=") {
            exdate
                .split(';')
                .find(|p| p.starts_with("TZID="))
                .and_then(|p| p.strip_prefix("TZID="))
                .and_then(|tz_part| tz_part.split(':').next())
                .and_then(parse_ical_timezone)
                .unwrap_or(tz)
        } else {
            tz
        };

        // Format EXDATE for rrule crate
        if exdate_tz == Tz::UTC || exdate_val.ends_with('Z') {
            rrule_str.push_str(&format!("\nEXDATE:{}Z", exdate_val.trim_end_matches('Z')));
        } else {
            rrule_str.push_str(&format!(
                "\nEXDATE;TZID={}:{}",
                exdate_tz,
                exdate_val.trim_end_matches('Z')
            ));
        }
    }

    // Parse the RRuleSet
    let rrule_set: RRuleSet = match rrule_str.parse() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    // Convert range to the rrule timezone
    let range_start_tz = range_start.with_timezone(&tz);
    let range_end_tz = range_end.with_timezone(&tz);

    // Get occurrences in the range (limit to 100 to avoid infinite loops)
    let result = rrule_set
        .after(range_start_tz)
        .before(range_end_tz)
        .all(100);

    // Convert to local time
    result
        .dates
        .into_iter()
        .map(|dt| dt.with_timezone(&Local))
        .collect()
}

/// Extract a meeting URL from the meeting's location or description fields
/// Checks location first (most common place for meeting links), then description
pub fn extract_meeting_url(meeting: &Meeting, patterns: &[String]) -> Option<String> {
    // Compile patterns, skipping any invalid ones
    let compiled: Vec<Regex> = patterns.iter().filter_map(|p| Regex::new(p).ok()).collect();

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    // Tests for parse_display_name
    #[test]
    fn test_parse_display_name_simple() {
        let data = "[Data Source]\nDisplayName=Work Calendar\nEnabled=true";
        assert_eq!(parse_display_name(data), Some("Work Calendar".to_string()));
    }

    #[test]
    fn test_parse_display_name_with_sections() {
        let data = "[Data Source]\nDisplayName=Personal\n[Calendar]\nBackendName=local";
        assert_eq!(parse_display_name(data), Some("Personal".to_string()));
    }

    #[test]
    fn test_parse_display_name_missing() {
        let data = "[Data Source]\nEnabled=true\n[Calendar]";
        assert_eq!(parse_display_name(data), None);
    }

    // Tests for parse_color
    #[test]
    fn test_parse_color_hex() {
        let data = "[Calendar]\nColor=#62a0ea\nBackendName=local";
        assert_eq!(parse_color(data), Some("#62a0ea".to_string()));
    }

    #[test]
    fn test_parse_color_missing() {
        let data = "[Calendar]\nBackendName=local";
        assert_eq!(parse_color(data), None);
    }

    // Tests for parse_ical_datetime
    #[test]
    fn test_parse_ical_datetime_local() {
        let now = Local::now();
        let result = parse_ical_datetime("20240221T123000", &now).unwrap();
        assert_eq!(result.year(), 2024);
        assert_eq!(result.month(), 2);
        assert_eq!(result.day(), 21);
        assert_eq!(result.hour(), 12);
        assert_eq!(result.minute(), 30);
    }

    #[test]
    fn test_parse_ical_datetime_utc() {
        let now = Local::now();
        let result = parse_ical_datetime("20240221T123000Z", &now).unwrap();
        assert_eq!(result.year(), 2024);
        assert_eq!(result.month(), 2);
        assert_eq!(result.day(), 21);
        // Hour may differ due to timezone conversion
    }

    #[test]
    fn test_parse_ical_datetime_with_tzid() {
        let now = Local::now();
        let result = parse_ical_datetime("TZID=America/Los_Angeles:20240221T123000", &now).unwrap();
        assert_eq!(result.year(), 2024);
        assert_eq!(result.month(), 2);
        assert_eq!(result.day(), 21);
    }

    #[test]
    fn test_parse_ical_datetime_date_only() {
        let now = Local::now();
        let result = parse_ical_datetime("20250527", &now).unwrap();
        assert_eq!(result.year(), 2025);
        assert_eq!(result.month(), 5);
        assert_eq!(result.day(), 27);
        assert_eq!(result.hour(), 0);
        assert_eq!(result.minute(), 0);
    }

    #[test]
    fn test_parse_ical_datetime_value_date() {
        let now = Local::now();
        let result = parse_ical_datetime("VALUE=DATE:20250527", &now).unwrap();
        assert_eq!(result.year(), 2025);
        assert_eq!(result.month(), 5);
        assert_eq!(result.day(), 27);
    }

    #[test]
    fn test_parse_ical_datetime_invalid() {
        let now = Local::now();
        assert!(parse_ical_datetime("invalid", &now).is_none());
        assert!(parse_ical_datetime("", &now).is_none());
    }

    // Helper to create a test meeting
    fn make_test_meeting(location: Option<&str>, description: Option<&str>) -> Meeting {
        Meeting {
            uid: "test-uid".to_string(),
            title: "Test Meeting".to_string(),
            start: Local::now(),
            end: Local::now() + chrono::Duration::hours(1),
            location: location.map(String::from),
            description: description.map(String::from),
            calendar_uid: "cal-uid".to_string(),
            is_all_day: false,
            attendance_status: AttendanceStatus::None,
        }
    }

    // Tests for extract_meeting_url
    #[test]
    fn test_extract_meeting_url_from_location() {
        let patterns = vec![r"https://meet\.google\.com/[a-z-]+".to_string()];
        let meeting = make_test_meeting(Some("https://meet.google.com/abc-defg-hij"), None);
        let url = extract_meeting_url(&meeting, &patterns);
        assert_eq!(
            url,
            Some("https://meet.google.com/abc-defg-hij".to_string())
        );
    }

    #[test]
    fn test_extract_meeting_url_from_description() {
        let patterns = vec![r"https://zoom\.us/j/\d+".to_string()];
        let meeting = make_test_meeting(
            Some("Conference Room A"),
            Some("Join: https://zoom.us/j/123456789"),
        );
        let url = extract_meeting_url(&meeting, &patterns);
        assert_eq!(url, Some("https://zoom.us/j/123456789".to_string()));
    }

    #[test]
    fn test_extract_meeting_url_location_priority() {
        let patterns = vec![r"https://meet\.google\.com/[a-z-]+".to_string()];
        let meeting = make_test_meeting(
            Some("https://meet.google.com/loc-ation"),
            Some("https://meet.google.com/desc-ription"),
        );
        let url = extract_meeting_url(&meeting, &patterns);
        // Location should be checked first
        assert_eq!(url, Some("https://meet.google.com/loc-ation".to_string()));
    }

    #[test]
    fn test_extract_meeting_url_no_match() {
        let patterns = vec![r"https://meet\.google\.com/[a-z-]+".to_string()];
        let meeting = make_test_meeting(Some("Conference Room B"), None);
        assert!(extract_meeting_url(&meeting, &patterns).is_none());
    }

    #[test]
    fn test_extract_meeting_url_empty_patterns() {
        let patterns: Vec<String> = vec![];
        let meeting = make_test_meeting(Some("https://meet.google.com/abc-def"), None);
        assert!(extract_meeting_url(&meeting, &patterns).is_none());
    }

    // Tests for get_physical_location
    #[test]
    fn test_get_physical_location_room() {
        let patterns = vec![r"https://meet\.google\.com/[a-z-]+".to_string()];
        let meeting = make_test_meeting(Some("Conference Room A"), None);
        assert_eq!(
            get_physical_location(&meeting, &patterns),
            Some("Conference Room A".to_string())
        );
    }

    #[test]
    fn test_get_physical_location_url_only() {
        let patterns = vec![r"https://meet\.google\.com/[a-z-]+".to_string()];
        let meeting = make_test_meeting(Some("https://meet.google.com/abc-defg-hij"), None);
        assert!(get_physical_location(&meeting, &patterns).is_none());
    }

    #[test]
    fn test_get_physical_location_generic_url() {
        let patterns: Vec<String> = vec![];
        let meeting = make_test_meeting(Some("https://example.com/meeting"), None);
        assert!(get_physical_location(&meeting, &patterns).is_none());
    }
}
