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


#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    /// Calendar UIDs that are enabled for display.
    /// Empty list means all calendars are enabled.
    pub enabled_calendar_uids: Vec<String>,
    /// How to display the meeting time in the panel.
    pub display_format: DisplayFormat,
}
