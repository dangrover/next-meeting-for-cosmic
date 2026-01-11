// SPDX-License-Identifier: GPL-3.0-only

mod app;
mod calendar;
mod config;
mod formatting;
mod i18n;
mod widgets;

fn main() -> cosmic::iced::Result {
    // Check for --join-next flag
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--join-next") {
        std::process::exit(join_next_meeting());
    }

    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Starts the applet's event loop with `()` as the application's flags.
    cosmic::applet::run::<app::AppModel>(())
}

/// Join the next upcoming meeting by opening its URL.
/// Used for keyboard shortcut integration.
/// Returns 0 on success, 1 if no meeting or no URL found.
fn join_next_meeting() -> i32 {
    use cosmic::cosmic_config::CosmicConfigEntry;

    const APP_ID: &str = "com.dangrover.next-meeting-app";

    // Load config to get enabled calendars and URL patterns
    let config = cosmic::cosmic_config::Config::new(APP_ID, config::Config::VERSION)
        .ok()
        .map(|ctx| config::Config::get_entry(&ctx).unwrap_or_else(|(_e, c)| c))
        .unwrap_or_default();

    // Build tokio runtime to run async calendar fetch
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let meetings = rt.block_on(calendar::get_upcoming_meetings(
        &config.enabled_calendar_uids,
        1, // Just need the first meeting
        &config.additional_emails,
    ));

    // Get the first meeting
    let Some(meeting) = meetings.first() else {
        return 1; // No meetings found
    };

    // Extract meeting URL
    let Some(url) = calendar::extract_meeting_url(meeting, &config.meeting_url_patterns) else {
        return 1; // No URL in meeting
    };

    // Open the URL (exit 0 on success, 1 on failure)
    i32::from(!app::open_url(&url))
}
