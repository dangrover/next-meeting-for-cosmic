// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry};
use serde::{Deserialize, Serialize};

/// How to display the meeting time in the panel
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum DisplayFormat {
    /// Show day and time (e.g., "Fri 9:30: All Hands")
    #[default]
    DayAndTime,
    /// Show relative time (e.g., "In 2d 3h: All Hands")
    Relative,
    /// Legacy: treated as DayAndTime
    TitleOnly,
    /// Legacy: treated as DayAndTime
    TimeOnly,
}

/// When to show the Join button
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum JoinButtonVisibility {
    /// Never show join button
    Hide,
    /// Always show join button (when URL available)
    Show,
    /// Show join button only when meeting is same day
    ShowIfSameDay,
    /// Show join button only when meeting is within 30 minutes
    ShowIf30m,
    /// Show join button only when meeting is within 15 minutes
    #[default]
    ShowIf15m,
    /// Show join button only when meeting is within 5 minutes
    ShowIf5m,
}

/// When to show the physical location
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum LocationVisibility {
    /// Never show location
    Hide,
    /// Always show location (when available)
    #[default]
    Show,
    /// Show location only when meeting is same day
    ShowIfSameDay,
    /// Show location only when meeting is within 30 minutes
    ShowIf30m,
    /// Show location only when meeting is within 15 minutes
    ShowIf15m,
    /// Show location only when meeting is within 5 minutes
    ShowIf5m,
}


#[derive(Debug, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    /// Calendar UIDs that are enabled for display.
    /// Empty list means all calendars are enabled.
    pub enabled_calendar_uids: Vec<String>,
    /// How to display the meeting time in the panel.
    pub display_format: DisplayFormat,
    /// Number of upcoming events to show in the popup (0-10).
    pub upcoming_events_count: u8,
    /// When to show the Join button in the popup.
    pub popup_join_button: JoinButtonVisibility,
    /// When to show the Join button in the panel.
    pub panel_join_button: JoinButtonVisibility,
    /// When to show the physical location in the popup.
    pub popup_location: LocationVisibility,
    /// When to show the physical location in the panel.
    pub panel_location: LocationVisibility,
    /// Regex patterns to detect meeting URLs in location/description.
    pub meeting_url_patterns: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled_calendar_uids: Vec::new(),
            display_format: DisplayFormat::default(),
            upcoming_events_count: 3,
            popup_join_button: JoinButtonVisibility::ShowIfSameDay,
            panel_join_button: JoinButtonVisibility::ShowIf15m,
            popup_location: LocationVisibility::default(),
            panel_location: LocationVisibility::default(),
            meeting_url_patterns: vec![
                // Google Meet
                r"https://meet\.google\.com/[a-z-]+".to_string(),
                // Zoom
                r"https://[a-z0-9]+\.zoom\.us/j/[0-9]+".to_string(),
                // Microsoft Teams
                r"https://teams\.microsoft\.com/l/meetup-join/[^\s]+".to_string(),
                r"https://teams\.live\.com/meet/[^\s]+".to_string(),
                // Webex
                r"https://[a-z0-9]+\.webex\.com/[^\s]+/j\.php\?MTID=[^\s]+".to_string(),
                r"https://[a-z0-9]+\.webex\.com/meet/[^\s]+".to_string(),
            ],
        }
    }
}
