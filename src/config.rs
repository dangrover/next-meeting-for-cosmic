// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry};

#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    /// Calendar UIDs that are enabled for display.
    /// Empty list means all calendars are enabled.
    pub enabled_calendar_uids: Vec<String>,
}
