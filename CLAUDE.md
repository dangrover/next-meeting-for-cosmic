# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Meeting is a COSMIC desktop applet that displays the next upcoming meeting in the system panel or dock. It integrates with GNOME Evolution Data Server via D-Bus to fetch calendar events.

- **Language**: Rust (Edition 2024)
- **Framework**: libcosmic (Pop!_OS COSMIC desktop)
- **App ID**: `com.dangrover.next-meeting-app`

## Build Commands

```bash
just                  # Build release (default)
just build-debug      # Debug build
just run              # Build and run for testing
just check            # Run clippy linter (pedantic)
just install          # Install to ~/.local
```

## Architecture

### Core Modules

- **main.rs**: Entry point - initializes i18n and launches COSMIC applet runtime
- **app.rs**: Application model implementing `cosmic::Application` trait with message-based updates
- **calendar.rs**: D-Bus integration with Evolution Data Server for calendar queries
- **config.rs**: Configuration using `cosmic_config` derive macros
- **i18n.rs**: Fluent-based localization via `i18n-embed`

### Data Flow

1. COSMIC panel launches applet via desktop entry (`X-CosmicApplet=true`)
2. `AppModel::init()` loads config and fetches initial meeting
3. Background subscription refreshes meetings every 60 seconds
4. D-Bus queries Evolution Data Server → parses iCalendar → returns next meeting

### Calendar Integration

The app reads calendar sources from `~/.config/evolution/sources/*.source`, opens each via D-Bus (`org.gnome.evolution.dataserver.Calendar8`), fetches events as iCalendar objects, parses them, filters to future events, and returns the soonest.

### COSMIC Application Pattern

Messages flow through `update()`:
- `TogglePopup` / `PopupClosed` - popup visibility
- `MeetingUpdated(Option<Meeting>)` - new calendar data
- `UpdateConfig(Config)` - config changes

Subscriptions run in background: calendar refresh (60s interval) and config watcher.

## Localization

Translations use Fluent format in `i18n/<lang>/meeting.ftl`. Add new languages by copying `i18n/en/` directory. Use `fl!("message-id")` macro in code.

## Key Dependencies

- `libcosmic` - COSMIC desktop framework (git dependency)
- `zbus` - D-Bus communication
- `ical` - iCalendar parsing
- `tokio` - Async runtime
- `chrono` - DateTime handling
