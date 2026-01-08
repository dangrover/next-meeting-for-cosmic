// SPDX-License-Identifier: GPL-3.0-only
//
// Debug utility to explore Evolution Data Server calendar data.
// Run with: cargo run --bin debug_calendars

use std::collections::HashMap;
use zbus::{Connection, zvariant::{OwnedObjectPath, OwnedValue, Value}};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::session().await?;

    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("list") => list_calendars(&conn).await?,
        Some("events") => {
            let uid = args.get(2).ok_or("Usage: debug_calendars events <calendar_uid> [limit]")?;
            let limit = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
            show_events(&conn, uid, limit).await?;
        }
        Some("raw") => {
            let uid = args.get(2).ok_or("Usage: debug_calendars raw <calendar_uid> [limit]")?;
            let limit = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5);
            show_raw_events(&conn, uid, limit).await?;
        }
        _ => {
            println!("Calendar Debug Utility");
            println!();
            println!("Usage:");
            println!("  cargo run --bin debug_calendars list              - List all calendar sources");
            println!("  cargo run --bin debug_calendars events <uid> [n]  - Show n parsed events from calendar");
            println!("  cargo run --bin debug_calendars raw <uid> [n]     - Show n raw ICS objects from calendar");
        }
    }

    Ok(())
}

async fn list_calendars(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let proxy = zbus::Proxy::new(
        conn,
        "org.gnome.evolution.dataserver.Sources5",
        "/org/gnome/evolution/dataserver/SourceManager",
        "org.freedesktop.DBus.ObjectManager",
    ).await?;

    let reply = proxy.call_method("GetManagedObjects", &()).await?;
    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> = reply.body()?;

    println!("{:<45} {:<30} {}", "UID", "Display Name", "Type");
    println!("{}", "-".repeat(90));

    for (_path, interfaces) in objects {
        if let Some(props) = interfaces.get("org.gnome.evolution.dataserver.Source") {
            let uid = props.get("UID").and_then(|v| {
                if let Some(Value::Str(s)) = v.downcast_ref::<Value>() {
                    Some(s.to_string())
                } else { None }
            });

            let data = props.get("Data").and_then(|v| {
                if let Some(Value::Str(s)) = v.downcast_ref::<Value>() {
                    Some(s.to_string())
                } else { None }
            });

            if let (Some(uid), Some(data)) = (uid, data) {
                let display_name = parse_field(&data, "DisplayName").unwrap_or_else(|| uid.clone());
                let identity = parse_field(&data, "Identity").unwrap_or_default();

                let source_type = if data.contains("[Calendar]") {
                    if identity.starts_with("tasks::") {
                        "Task List"
                    } else {
                        "Calendar"
                    }
                } else if data.contains("[Address Book]") {
                    "Contacts"
                } else if data.contains("[Mail Account]") {
                    "Mail"
                } else if data.contains("[Collection]") {
                    "Collection"
                } else {
                    "Other"
                };

                // Only show calendars and task lists
                if data.contains("[Calendar]") {
                    println!("{:<45} {:<30} {}", uid, display_name, source_type);
                }
            }
        }
    }

    Ok(())
}

async fn show_events(conn: &Connection, uid: &str, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let events = get_calendar_events(conn, uid).await?;

    println!("Found {} total events in calendar", events.len());
    println!();

    let now = chrono::Local::now();
    let mut future_events: Vec<_> = events.iter()
        .filter_map(|ics| parse_event(ics))
        .filter(|e| e.start > now)
        .collect();

    future_events.sort_by_key(|e| e.start);

    println!("Future events ({} found):", future_events.len());
    println!("{:<25} {}", "Start", "Title");
    println!("{}", "-".repeat(80));

    for event in future_events.iter().take(limit) {
        println!("{:<25} {}", event.start.format("%Y-%m-%d %H:%M"), event.title);
    }

    Ok(())
}

async fn show_raw_events(conn: &Connection, uid: &str, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let events = get_calendar_events(conn, uid).await?;

    println!("Showing {} of {} raw ICS objects:\n", limit.min(events.len()), events.len());

    for (i, ics) in events.iter().take(limit).enumerate() {
        println!("=== Event {} ===", i + 1);
        // Show full raw ICS (first 20 lines)
        for (j, line) in ics.lines().enumerate() {
            if j < 20 {
                println!("  {}", line);
            }
        }
        if ics.lines().count() > 20 {
            println!("  ... ({} more lines)", ics.lines().count() - 20);
        }

        // Also show what ical crate parses (wrap in VCALENDAR if needed)
        let wrapped = if ics.trim().starts_with("BEGIN:VEVENT") {
            format!("BEGIN:VCALENDAR\nVERSION:2.0\n{}\nEND:VCALENDAR", ics)
        } else {
            ics.clone()
        };
        let parse_result = ical::parser::ical::IcalParser::new(wrapped.as_bytes()).next();
        match parse_result {
            Some(Ok(cal)) => {
                println!("  [ical crate] parsed {} events", cal.events.len());
                for event in cal.events {
                    for prop in &event.properties {
                        if prop.name == "DTSTART" {
                            println!("  [ical crate] DTSTART value: {:?}", prop.value);
                        }
                    }
                }
            }
            Some(Err(e)) => println!("  [ical crate] parse error: {}", e),
            None => println!("  [ical crate] no calendar found"),
        }
        println!();
    }

    Ok(())
}

async fn get_calendar_events(conn: &Connection, uid: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let factory = zbus::Proxy::new(
        conn,
        "org.gnome.evolution.dataserver.Calendar8",
        "/org/gnome/evolution/dataserver/CalendarFactory",
        "org.gnome.evolution.dataserver.CalendarFactory",
    ).await?;

    let reply = factory.call_method("OpenCalendar", &(uid,)).await?;
    let (calendar_path, bus_name): (String, String) = reply.body()?;

    let calendar = zbus::Proxy::new(
        conn,
        bus_name.as_str(),
        calendar_path.as_str(),
        "org.gnome.evolution.dataserver.Calendar",
    ).await?;

    let reply = calendar.call_method("GetObjectList", &("",)).await?;
    let events: Vec<String> = reply.body()?;

    Ok(events)
}

struct ParsedEvent {
    title: String,
    start: chrono::DateTime<chrono::Local>,
}

fn parse_event(ics: &str) -> Option<ParsedEvent> {
    let mut title = None;
    let mut start = None;

    for line in ics.lines() {
        let line = line.trim();
        if line.starts_with("SUMMARY") {
            title = line.split(':').nth(1).map(|s| s.to_string());
        } else if line.starts_with("DTSTART") {
            start = parse_ical_datetime(line);
        }
    }

    Some(ParsedEvent {
        title: title.unwrap_or_else(|| "Untitled".to_string()),
        start: start?,
    })
}

fn parse_ical_datetime(line: &str) -> Option<chrono::DateTime<chrono::Local>> {
    use chrono::{NaiveDateTime, TimeZone, Local};

    // Extract the value part (after the colon)
    let value = line.split(':').last()?;
    let value = value.trim();

    // Handle UTC times (ending with Z)
    if value.ends_with('Z') {
        let value = &value[..value.len()-1];
        if value.len() >= 15 {
            let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
            return Some(chrono::Utc.from_utc_datetime(&naive).with_timezone(&Local));
        }
    }

    // Handle local times (YYYYMMDDTHHMMSS)
    if value.len() >= 15 && value.contains('T') {
        let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
        return Local.from_local_datetime(&naive).single();
    }

    // Handle date-only (YYYYMMDD)
    if value.len() == 8 {
        let naive = NaiveDateTime::parse_from_str(&format!("{}T000000", value), "%Y%m%dT%H%M%S").ok()?;
        return Local.from_local_datetime(&naive).single();
    }

    None
}

fn parse_field(data: &str, field: &str) -> Option<String> {
    let prefix = format!("{}=", field);
    for line in data.lines() {
        let line = line.trim();
        if line.starts_with(&prefix) {
            return Some(line.strip_prefix(&prefix)?.to_string());
        }
    }
    None
}
