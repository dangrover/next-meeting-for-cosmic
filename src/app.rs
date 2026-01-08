// SPDX-License-Identifier: GPL-3.0-only

use crate::calendar::{CalendarInfo, Meeting, extract_meeting_url, get_physical_location};
use crate::config::{Config, DisplayFormat, JoinButtonVisibility, LocationVisibility};
use crate::fl;
use crate::formatting::{
    format_backend_name, format_last_updated, format_panel_time, format_relative_time,
    format_time, parse_hex_color,
};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::cosmic_theme;
use cosmic::iced::{Length, Limits, Subscription, window::Id};
use cosmic::iced_core::id;
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::prelude::*;
use cosmic::widget;
use futures_util::SinkExt;

/// Get display format labels for the dropdown (must be called at runtime for localization)
fn display_format_options() -> Vec<String> {
    vec![
        fl!("display-format-day-time"),
        fl!("display-format-relative"),
    ]
}

/// Get theme spacing values
fn spacing() -> cosmic_theme::Spacing {
    cosmic::theme::spacing()
}

/// Generate a unique ID for an email input field
fn email_input_id(idx: usize) -> id::Id {
    id::Id::new(format!("email_input_{}", idx))
}

/// Secondary text style for dimmed/muted text appearance
fn secondary_text_style(theme: &cosmic::Theme) -> cosmic::iced_widget::text::Style {
    cosmic::iced_widget::text::Style {
        color: Some(theme.cosmic().palette.neutral_6.into()),
    }
}

/// Featured item button style: transparent background, rounded rect hover matching menu items
fn featured_button_style() -> cosmic::theme::Button {
    cosmic::theme::Button::Custom {
        active: Box::new(|_focused, theme| {
            let cosmic = theme.cosmic();
            cosmic::widget::button::Style {
                background: None,
                text_color: Some(cosmic.on_bg_color().into()),
                icon_color: Some(cosmic.on_bg_color().into()),
                border_radius: cosmic.corner_radii.radius_s.into(),
                ..Default::default()
            }
        }),
        disabled: Box::new(|theme| {
            let cosmic = theme.cosmic();
            cosmic::widget::button::Style {
                background: None,
                text_color: Some(cosmic.on_bg_color().into()),
                icon_color: Some(cosmic.on_bg_color().into()),
                border_radius: cosmic.corner_radii.radius_s.into(),
                ..Default::default()
            }
        }),
        hovered: Box::new(|_focused, theme| {
            let cosmic = theme.cosmic();
            // Use text_button.hover to match AppletMenu hover color
            cosmic::widget::button::Style {
                background: Some(cosmic::iced::Background::Color(
                    cosmic.text_button.hover.into(),
                )),
                text_color: Some(cosmic.on_bg_color().into()),
                icon_color: Some(cosmic.on_bg_color().into()),
                border_radius: cosmic.corner_radii.radius_s.into(),
                ..Default::default()
            }
        }),
        pressed: Box::new(|_focused, theme| {
            let cosmic = theme.cosmic();
            // Use text_button.pressed to match AppletMenu pressed color
            cosmic::widget::button::Style {
                background: Some(cosmic::iced::Background::Color(
                    cosmic.text_button.pressed.into(),
                )),
                text_color: Some(cosmic.on_bg_color().into()),
                icon_color: Some(cosmic.on_bg_color().into()),
                border_radius: cosmic.corner_radii.radius_s.into(),
                ..Default::default()
            }
        }),
    }
}

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
#[derive(Default)]
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// The popup id.
    popup: Option<Id>,
    /// Configuration data that persists between application runs.
    config: Config,
    /// Upcoming meetings to display.
    upcoming_meetings: Vec<Meeting>,
    /// Available calendars from Evolution Data Server.
    available_calendars: Vec<CalendarInfo>,
    /// Config context for saving changes.
    config_context: Option<cosmic_config::Config>,
    /// Current page in popup navigation
    current_page: PopupPage,
    /// Whether a calendar refresh is in progress.
    is_refreshing: bool,
}

/// Navigation state for popup pages
#[derive(Debug, Default, Clone, PartialEq)]
pub enum PopupPage {
    #[default]
    Main,
    Settings,
    Calendars,
    RefreshSettings,
    JoinButtonSettings,
    LocationSettings,
    CalendarIndicatorSettings,
    EventsToShowSettings,
    EmailSettings,
    About,
}

impl AppModel {
    /// Get meetings filtered by current settings (all-day events, attendance status)
    fn filtered_meetings(&self) -> Vec<&Meeting> {
        use crate::calendar::AttendanceStatus;
        use crate::config::EventStatusFilter;

        self.upcoming_meetings
            .iter()
            .filter(|m| {
                // Filter out all-day events if disabled
                if !self.config.show_all_day_events && m.is_all_day {
                    return false;
                }

                // Filter by attendance status
                match self.config.event_status_filter {
                    EventStatusFilter::All => true,
                    EventStatusFilter::Accepted => {
                        matches!(
                            m.attendance_status,
                            AttendanceStatus::Accepted | AttendanceStatus::None
                        )
                    }
                    EventStatusFilter::AcceptedOrTentative => {
                        matches!(
                            m.attendance_status,
                            AttendanceStatus::Accepted
                                | AttendanceStatus::Tentative
                                | AttendanceStatus::None
                        )
                    }
                }
            })
            .collect()
    }

    /// Get UIDs of enabled calendars that are valid meeting sources.
    /// Filters out non-meeting calendars (contacts, weather, birthdays).
    fn enabled_meeting_source_uids(&self) -> Vec<String> {
        // If we don't have the calendars list yet, fall back to config
        // (non-meeting calendars won't have VEVENT data anyway)
        if self.available_calendars.is_empty() {
            return self.config.enabled_calendar_uids.clone();
        }

        // If enabled_calendar_uids is empty, all meeting-source calendars are enabled
        if self.config.enabled_calendar_uids.is_empty() {
            self.available_calendars
                .iter()
                .filter(|c| c.is_meeting_source())
                .map(|c| c.uid.clone())
                .collect()
        } else {
            // Filter the enabled list to only include meeting sources
            self.config
                .enabled_calendar_uids
                .iter()
                .filter(|uid| {
                    self.available_calendars
                        .iter()
                        .find(|c| &c.uid == *uid)
                        .is_some_and(|c| c.is_meeting_source())
                })
                .cloned()
                .collect()
        }
    }

    /// Main popup page showing meeting info and settings nav
    fn view_main_page(&self) -> Element<'_, Message> {
        let space = spacing();

        let mut content = widget::column::with_capacity(8)
            .padding([space.space_xxs, space.space_none])
            .width(Length::Fill);

        let filtered = self.filtered_meetings();
        if let Some(meeting) = filtered.first() {
            use chrono::Local;
            let now = Local::now();
            let minutes_until = meeting.start.signed_duration_since(now).num_minutes();
            let is_same_day = meeting.start.date_naive() == now.date_naive();
            let time_str = format_time(&meeting.start, true);

            // Check for meeting URL based on popup join button visibility settings
            let show_join = match self.config.popup_join_button {
                JoinButtonVisibility::Hide => false,
                JoinButtonVisibility::Show => true,
                JoinButtonVisibility::ShowIfSameDay => is_same_day,
                JoinButtonVisibility::ShowIf30m => minutes_until <= 30,
                JoinButtonVisibility::ShowIf15m => minutes_until <= 15,
                JoinButtonVisibility::ShowIf5m => minutes_until <= 5,
            };
            let meeting_url = if show_join {
                extract_meeting_url(meeting, &self.config.meeting_url_patterns)
            } else {
                None
            };

            // Check for physical location based on popup location visibility settings
            let show_location = match self.config.popup_location {
                LocationVisibility::Hide => false,
                LocationVisibility::Show => true,
                LocationVisibility::ShowIfSameDay => is_same_day,
                LocationVisibility::ShowIf30m => minutes_until <= 30,
                LocationVisibility::ShowIf15m => minutes_until <= 15,
                LocationVisibility::ShowIf5m => minutes_until <= 5,
            };
            let physical_location = if show_location {
                get_physical_location(meeting, &self.config.meeting_url_patterns)
            } else {
                None
            };

            // Next meeting content block with optional Join button
            // Uses custom style: transparent background, rounded rect on hover
            let next_meeting_uid = meeting.uid.clone();
            let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);

            // Build title row with optional calendar indicator
            let title_row = if self.config.popup_calendar_indicator {
                let mut row = widget::row::with_capacity(2)
                    .spacing(space.space_xxs)
                    .align_y(cosmic::iced::Alignment::Center);
                if let Some(dot) = calendar_color_dot::<Message>(
                    &meeting.calendar_uid,
                    &self.available_calendars,
                    10.0,
                    Some(widget::tooltip::Position::Top),
                ) {
                    row = row.push(dot);
                }
                row.push(widget::text::title4(&meeting.title))
            } else {
                widget::row::with_capacity(1).push(widget::text::title4(&meeting.title))
            };

            // Build meeting info column with title, time, and optional location
            let mut meeting_column = widget::column::with_capacity(3)
                .push(title_row)
                .push(widget::text::body(time_str).class(secondary_text))
                .spacing(space.space_xxxs)
                .width(Length::Fill);

            if let Some(location) = physical_location {
                meeting_column =
                    meeting_column.push(widget::text::body(location).class(secondary_text));
            }

            let meeting_info = widget::button::custom(meeting_column)
                .class(featured_button_style())
                .padding([space.space_xxs, space.space_xs])
                .width(Length::Fill)
                .on_press(Message::OpenEvent(next_meeting_uid));

            if let Some(url) = meeting_url {
                // Row with meeting info and Join button (with horizontal padding)
                content = content.push(
                    widget::row::with_capacity(2)
                        .push(meeting_info)
                        .push(
                            widget::button::suggested(fl!("join"))
                                .on_press(Message::OpenMeetingUrl(url)),
                        )
                        .align_y(cosmic::iced::Alignment::Center)
                        .spacing(space.space_xs)
                        .width(Length::Fill)
                        .apply(widget::container)
                        .padding([0, space.space_s]),
                );
            } else {
                // Wrap in container with horizontal padding
                content = content.push(
                    meeting_info
                        .apply(widget::container)
                        .padding([0, space.space_s]),
                );
            }

            // Upcoming events section
            let upcoming_count = self.config.upcoming_events_count as usize;
            if upcoming_count > 0 && filtered.len() > 1 {
                // Divider before "Upcoming" section
                content = content.push(
                    cosmic::applet::padded_control(widget::divider::horizontal::default())
                        .padding([space.space_xxs, space.space_s]),
                );

                // "Upcoming" section heading
                content = content.push(cosmic::applet::padded_control(widget::text::heading(fl!(
                    "upcoming"
                ))));

                let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
                for meeting in filtered.iter().skip(1).take(upcoming_count) {
                    let title = if meeting.title.len() > 25 {
                        format!("{}...", &meeting.title[..22])
                    } else {
                        meeting.title.clone()
                    };
                    let time_str = format_time(&meeting.start, false);
                    let uid = meeting.uid.clone();

                    // Build row with optional calendar indicator
                    let mut row = widget::row::with_capacity(4)
                        .spacing(space.space_xs)
                        .align_y(cosmic::iced::Alignment::Center)
                        .width(Length::Fill);

                    if self.config.popup_calendar_indicator
                        && let Some(dot) = calendar_color_dot::<Message>(
                            &meeting.calendar_uid,
                            &self.available_calendars,
                            8.0,
                            None,
                        )
                    {
                        row = row.push(dot);
                    }

                    row = row
                        .push(widget::text::body(title))
                        .push(widget::horizontal_space())
                        .push(widget::text::body(time_str).class(secondary_text));

                    content = content
                        .push(cosmic::applet::menu_button(row).on_press(Message::OpenEvent(uid)));
                }
            }
        } else if self.available_calendars.is_empty() {
            // No calendars configured - show prominent centered message
            let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);

            let no_cal_content = widget::column::with_capacity(3)
                .spacing(space.space_xxs)
                .align_x(cosmic::iced::Alignment::Center)
                .push(widget::icon::from_name("dialog-warning-symbolic").size(space.space_l))
                .push(widget::text::title4(fl!("no-calendars")))
                .push(widget::text::body(fl!("no-calendars-description")).class(secondary_text));

            content = content.push(
                widget::container(no_cal_content)
                    .padding([space.space_s, space.space_s])
                    .width(Length::Fill)
                    .align_x(cosmic::iced::alignment::Horizontal::Center),
            );
        } else {
            // Calendars exist but no upcoming meetings
            content = content.push(cosmic::applet::padded_control(widget::text::body(fl!(
                "no-meetings"
            ))));
        }

        // Divider before bottom actions
        content = content.push(
            cosmic::applet::padded_control(widget::divider::horizontal::default())
                .padding([space.space_xxs, space.space_s]),
        );

        // Bottom actions section (Open calendar + Settings)
        content = content.push(
            cosmic::applet::menu_button(
                widget::row::with_capacity(3)
                    .push(widget::icon::from_name("office-calendar-symbolic").size(space.space_m))
                    .push(widget::text::body(fl!("open-calendar")))
                    .push(widget::horizontal_space())
                    .spacing(space.space_xs)
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            .on_press(Message::OpenCalendar),
        );

        content = content.push(
            cosmic::applet::menu_button(
                widget::row::with_capacity(3)
                    .push(
                        widget::icon::from_name("preferences-system-symbolic").size(space.space_m),
                    )
                    .push(widget::text::body(fl!("settings")))
                    .push(widget::horizontal_space())
                    .spacing(space.space_xs)
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            .on_press(Message::Navigate(PopupPage::Settings)),
        );

        content.into()
    }

    /// Settings page with back button
    fn view_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(2)
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .width(Length::Fill);

        // Back button header
        content = content.push(
            widget::column::with_capacity(2)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("back"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Main)),
                )
                .push(widget::text::title4(fl!("settings")))
                .spacing(space.space_xxs),
        );

        // Refresh status summary
        let refresh_summary = if self.config.auto_refresh_enabled {
            fl!(
                "refresh-summary-on",
                interval = self.config.auto_refresh_interval_minutes
            )
        } else {
            fl!("refresh-summary-off")
        };

        // Calendars count for summary
        let total = self.available_calendars.len();
        let calendar_summary = if total == 0 {
            fl!("calendars-none")
        } else {
            let enabled = if self.config.enabled_calendar_uids.is_empty() {
                total
            } else {
                // Count only UIDs that still exist in available calendars (filter stale UIDs)
                self.config
                    .enabled_calendar_uids
                    .iter()
                    .filter(|uid| self.available_calendars.iter().any(|c| &c.uid == *uid))
                    .count()
            };
            fl!("calendars-enabled", enabled = enabled, total = total)
        };

        // Display format dropdown index
        let format_idx = match self.config.display_format {
            DisplayFormat::DayAndTime => Some(0),
            DisplayFormat::Relative => Some(1),
            _ => Some(0),
        };

        // Join button status summary
        let join_status = match (
            &self.config.panel_join_button,
            &self.config.popup_join_button,
        ) {
            (JoinButtonVisibility::Hide, JoinButtonVisibility::Hide) => fl!("status-off"),
            (JoinButtonVisibility::Hide, _) => fl!("status-popup"),
            (_, JoinButtonVisibility::Hide) => fl!("status-panel"),
            _ => fl!("status-both"),
        };

        // Location status summary
        let location_status = match (&self.config.panel_location, &self.config.popup_location) {
            (LocationVisibility::Hide, LocationVisibility::Hide) => fl!("status-off"),
            (LocationVisibility::Hide, _) => fl!("status-popup"),
            (_, LocationVisibility::Hide) => fl!("status-panel"),
            _ => fl!("status-both"),
        };

        // Calendar indicator status summary
        let indicator_status = match (
            self.config.panel_calendar_indicator,
            self.config.popup_calendar_indicator,
        ) {
            (false, false) => fl!("status-off"),
            (false, true) => fl!("status-popup"),
            (true, false) => fl!("status-panel"),
            (true, true) => fl!("status-both"),
        };

        // Filter summary for display
        use crate::config::EventStatusFilter;
        let has_allday_filter = !self.config.show_all_day_events;
        let has_status_filter = self.config.event_status_filter != EventStatusFilter::All;

        let filter_summary = match (has_allday_filter, has_status_filter) {
            (false, false) => fl!("filter-summary-all"),
            (true, false) => fl!("filter-summary-no-all-day"),
            (false, true) => match self.config.event_status_filter {
                EventStatusFilter::Accepted => fl!("filter-summary-accepted"),
                EventStatusFilter::AcceptedOrTentative => fl!("filter-summary-tentative"),
                _ => fl!("filter-summary-all"),
            },
            (true, true) => {
                let status = match self.config.event_status_filter {
                    EventStatusFilter::Accepted => fl!("filter-summary-accepted"),
                    EventStatusFilter::AcceptedOrTentative => fl!("filter-summary-tentative"),
                    _ => fl!("filter-summary-all"),
                };
                fl!(
                    "filter-summary-combo",
                    allday = fl!("filter-summary-no-all-day"),
                    status = status
                )
            }
        };

        // Calendars and filter section (its own group)
        let calendars_section = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Calendars
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("calendars-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(calendar_summary))
                                .push(
                                    widget::icon::from_name("go-next-symbolic").size(space.space_m),
                                )
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center),
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Calendars)),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Filter events
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("filter-events-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(filter_summary))
                                .push(
                                    widget::icon::from_name("go-next-symbolic").size(space.space_m),
                                )
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center),
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::EventsToShowSettings)),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Calendar sync
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("refresh-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(refresh_summary))
                                .push(
                                    widget::icon::from_name("go-next-symbolic").size(space.space_m),
                                )
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center),
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::RefreshSettings)),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );

        content = content.push(calendars_section);

        // More vertical spacing between sections
        content = content.push(widget::vertical_space().height(space.space_xs));

        // Display settings section
        let display_settings = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Display format
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("display-format-section")))
                    .push(widget::horizontal_space())
                    .push(widget::dropdown(
                        display_format_options(),
                        format_idx,
                        Message::SelectDisplayFormat,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Upcoming events count
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("upcoming-events-section")))
                    .push(widget::horizontal_space())
                    .push(widget::spin_button(
                        self.config.upcoming_events_count.to_string(),
                        self.config.upcoming_events_count as i32,
                        1,
                        0,
                        10,
                        Message::SetUpcomingEventsCount,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );

        content = content.push(display_settings);

        // More vertical spacing between sections
        content = content.push(widget::vertical_space().height(space.space_xs));

        // Meeting details section (join button, location, calendar indicator)
        let details_settings = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Join button settings
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("join-button-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(join_status))
                                .push(
                                    widget::icon::from_name("go-next-symbolic").size(space.space_m),
                                )
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center),
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::JoinButtonSettings)),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Physical location settings
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("location-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(location_status))
                                .push(
                                    widget::icon::from_name("go-next-symbolic").size(space.space_m),
                                )
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center),
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::LocationSettings)),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Calendar indicator settings
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("calendar-indicator-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(indicator_status))
                                .push(
                                    widget::icon::from_name("go-next-symbolic").size(space.space_m),
                                )
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center),
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::CalendarIndicatorSettings)),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );

        content = content.push(details_settings);

        // More vertical spacing before About section
        content = content.push(widget::vertical_space().height(space.space_xs));

        // About section
        let about_section = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            .add(
                widget::button::custom(
                    widget::row::with_capacity(2)
                        .push(widget::text::body(fl!("about")))
                        .push(widget::horizontal_space())
                        .push(widget::icon::from_name("go-next-symbolic").size(space.space_m))
                        .align_y(cosmic::iced::Alignment::Center)
                        .width(Length::Fill),
                )
                .class(cosmic::theme::Button::MenuRoot)
                .width(Length::Fill)
                .on_press(Message::Navigate(PopupPage::About)),
            );

        content = content.push(about_section);

        content.into()
    }

    /// Calendars selection page
    fn view_calendars_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(2 + self.available_calendars.len())
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .width(Length::Fill);

        // Back button header
        content = content.push(
            widget::column::with_capacity(2)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("settings"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Settings)),
                )
                .push(widget::text::title4(fl!("calendars-section")))
                .spacing(space.space_xxs),
        );

        // Calendar toggles in a single list_column
        let mut calendars_list =
            widget::list_column().list_item_padding([space.space_xxs, space.space_xs]);

        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);

        for calendar in &self.available_calendars {
            let is_meeting_source = calendar.is_meeting_source();
            let is_enabled = is_meeting_source
                && (self.config.enabled_calendar_uids.is_empty()
                    || self.config.enabled_calendar_uids.contains(&calendar.uid));

            let uid = calendar.uid.clone();

            // Build row with optional color indicator
            let mut row = widget::row::with_capacity(4)
                .spacing(space.space_xs)
                .align_y(cosmic::iced::Alignment::Center);

            // Add color circle if color is available
            if let Some(color) = &calendar.color
                && let Some(parsed_color) = parse_hex_color(color)
            {
                row = row.push(
                    widget::container(widget::Space::new(0, 0))
                        .width(Length::Fixed(12.0))
                        .height(Length::Fixed(12.0))
                        .class(cosmic::theme::Container::custom(move |_theme| {
                            cosmic::iced_widget::container::Style {
                                background: Some(cosmic::iced::Background::Color(parsed_color)),
                                border: cosmic::iced::Border {
                                    radius: 6.0.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }
                        })),
                );
            }

            // Calendar name and metadata in a column
            let mut name_col = widget::column::with_capacity(2)
                .spacing(space.space_xxxs);

            name_col = name_col.push(widget::text::body(&calendar.display_name));

            // Build secondary line with backend type and/or last updated time
            let backend_str = calendar.backend.as_ref().map(|b| format_backend_name(b));
            let updated_str = calendar.last_synced.as_ref().map(|s| format_last_updated(s));

            let secondary_line = match (backend_str, updated_str) {
                (Some(backend), Some(updated)) => Some(format!("{backend} · {updated}")),
                (Some(backend), None) => Some(backend.to_string()),
                (None, Some(updated)) => Some(updated),
                (None, None) => None,
            };

            if let Some(line) = secondary_line {
                name_col = name_col.push(widget::text::caption(line).class(secondary_text));
            }

            // Create toggler - disabled for non-meeting sources (contacts, weather, birthdays)
            let toggler = if is_meeting_source {
                widget::toggler(is_enabled)
                    .on_toggle(move |_| Message::ToggleCalendar(uid.clone()))
            } else {
                // Non-meeting sources: show disabled toggle (no on_toggle = non-interactive)
                widget::toggler(false)
            };

            row = row
                .push(name_col)
                .push(widget::horizontal_space())
                .push(toggler);

            calendars_list = calendars_list.add(row);
        }

        if self.available_calendars.is_empty() {
            // No calendars - show explanation
            let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
            content = content
                .push(widget::text::body(fl!("no-calendars-description")).class(secondary_text));
        } else {
            content = content.push(calendars_list);
        }

        content.into()
    }

    /// Join button settings page
    fn view_join_button_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(6 + self.config.meeting_url_patterns.len())
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .width(Length::Fill);

        // Back button header
        content = content.push(
            widget::column::with_capacity(2)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("settings"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Settings)),
                )
                .push(widget::text::title4(fl!("join-button-section")))
                .spacing(space.space_xxs),
        );

        // Dropdown options for join button visibility (6 options)
        let join_options = vec![
            fl!("join-hide"),
            fl!("join-show"),
            fl!("join-show-same-day"),
            fl!("join-show-30m"),
            fl!("join-show-15m"),
            fl!("join-show-5m"),
        ];

        // Panel join button dropdown index
        let panel_join_idx = match self.config.panel_join_button {
            JoinButtonVisibility::Hide => Some(0),
            JoinButtonVisibility::Show => Some(1),
            JoinButtonVisibility::ShowIfSameDay => Some(2),
            JoinButtonVisibility::ShowIf30m => Some(3),
            JoinButtonVisibility::ShowIf15m => Some(4),
            JoinButtonVisibility::ShowIf5m => Some(5),
        };

        // Popup join button dropdown index
        let popup_join_idx = match self.config.popup_join_button {
            JoinButtonVisibility::Hide => Some(0),
            JoinButtonVisibility::Show => Some(1),
            JoinButtonVisibility::ShowIfSameDay => Some(2),
            JoinButtonVisibility::ShowIf30m => Some(3),
            JoinButtonVisibility::ShowIf15m => Some(4),
            JoinButtonVisibility::ShowIf5m => Some(5),
        };

        // Join button visibility settings group
        let visibility_settings = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Panel join button
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("panel-join-button")))
                    .push(widget::horizontal_space())
                    .push(widget::dropdown(
                        join_options.clone(),
                        panel_join_idx,
                        Message::SetPanelJoinButton,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Popup join button
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("popup-join-button")))
                    .push(widget::horizontal_space())
                    .push(widget::dropdown(
                        join_options,
                        popup_join_idx,
                        Message::SetPopupJoinButton,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );

        content = content.push(visibility_settings);

        // Space before URL patterns section
        content = content.push(widget::vertical_space().height(space.space_xxs));

        // URL patterns section heading
        content = content.push(widget::text::heading(fl!("url-patterns")));

        // Pattern list as a grouped list
        let mut patterns_list =
            widget::list_column().list_item_padding([space.space_xxs, space.space_xs]);

        for (idx, pattern) in self.config.meeting_url_patterns.iter().enumerate() {
            patterns_list = patterns_list.add(
                widget::row::with_capacity(2)
                    .push(
                        widget::text_input("https://example.com/meeting/.*", pattern)
                            .on_input(move |s| Message::UpdatePattern(idx, s))
                            .width(Length::Fill),
                    )
                    .push(
                        widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
                            .extra_small()
                            .on_press(Message::RemovePattern(idx)),
                    )
                    .spacing(space.space_xs)
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );
        }

        content = content.push(patterns_list);

        // Add pattern button
        content = content
            .push(widget::button::standard(fl!("add-pattern")).on_press(Message::AddPattern));

        // Description at bottom with secondary color
        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
        content = content.push(widget::vertical_space().height(space.space_s));
        content = content
            .push(widget::text::caption(fl!("join-button-description")).class(secondary_text));
        content = content.push(widget::vertical_space().height(space.space_m));

        content.into()
    }

    /// Physical location settings page
    fn view_location_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(4)
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .width(Length::Fill);

        // Back button header
        content = content.push(
            widget::column::with_capacity(2)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("settings"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Settings)),
                )
                .push(widget::text::title4(fl!("location-section")))
                .spacing(space.space_xxs),
        );

        // Dropdown options for location visibility (6 options)
        let location_options = vec![
            fl!("location-hide"),
            fl!("location-show"),
            fl!("location-show-same-day"),
            fl!("location-show-30m"),
            fl!("location-show-15m"),
            fl!("location-show-5m"),
        ];

        // Panel location dropdown index
        let panel_location_idx = match self.config.panel_location {
            LocationVisibility::Hide => Some(0),
            LocationVisibility::Show => Some(1),
            LocationVisibility::ShowIfSameDay => Some(2),
            LocationVisibility::ShowIf30m => Some(3),
            LocationVisibility::ShowIf15m => Some(4),
            LocationVisibility::ShowIf5m => Some(5),
        };

        // Popup location dropdown index
        let popup_location_idx = match self.config.popup_location {
            LocationVisibility::Hide => Some(0),
            LocationVisibility::Show => Some(1),
            LocationVisibility::ShowIfSameDay => Some(2),
            LocationVisibility::ShowIf30m => Some(3),
            LocationVisibility::ShowIf15m => Some(4),
            LocationVisibility::ShowIf5m => Some(5),
        };

        // Location visibility settings group
        let visibility_settings = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Panel location
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("panel-location")))
                    .push(widget::horizontal_space())
                    .push(widget::dropdown(
                        location_options.clone(),
                        panel_location_idx,
                        Message::SetPanelLocation,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Popup location
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("popup-location")))
                    .push(widget::horizontal_space())
                    .push(widget::dropdown(
                        location_options,
                        popup_location_idx,
                        Message::SetPopupLocation,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );

        content = content.push(visibility_settings);

        // Description at bottom with secondary color
        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
        content = content.push(widget::vertical_space().height(space.space_s));
        content =
            content.push(widget::text::caption(fl!("location-description")).class(secondary_text));
        content = content.push(widget::vertical_space().height(space.space_m));

        content.into()
    }

    /// Calendar indicator settings page
    fn view_calendar_indicator_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(4)
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .width(Length::Fill);

        // Back button header
        content = content.push(
            widget::column::with_capacity(2)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("settings"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Settings)),
                )
                .push(widget::text::title4(fl!("calendar-indicator-section")))
                .spacing(space.space_xxs),
        );

        // Calendar indicator settings group
        let indicator_settings = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Panel indicator toggle
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("panel-indicator")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::toggler(self.config.panel_calendar_indicator)
                            .on_toggle(Message::SetPanelCalendarIndicator),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Popup indicator toggle
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("popup-indicator")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::toggler(self.config.popup_calendar_indicator)
                            .on_toggle(Message::SetPopupCalendarIndicator),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );

        content = content.push(indicator_settings);

        // Description at bottom with secondary color
        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
        content = content.push(widget::vertical_space().height(space.space_s));
        content = content.push(
            widget::text::caption(fl!("calendar-indicator-description")).class(secondary_text),
        );
        content = content.push(widget::vertical_space().height(space.space_m));

        content.into()
    }

    /// Filter events settings page
    fn view_events_to_show_settings_page(&self) -> Element<'_, Message> {
        use crate::config::EventStatusFilter;

        let space = spacing();
        let mut content = widget::column::with_capacity(6)
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .width(Length::Fill);

        // Back button header
        content = content.push(
            widget::column::with_capacity(2)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("settings"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Settings)),
                )
                .push(widget::text::title4(fl!("filter-events-section")))
                .spacing(space.space_xxs),
        );

        // Status filter dropdown options
        let status_options = vec![
            fl!("status-filter-all"),
            fl!("status-filter-accepted"),
            fl!("status-filter-accepted-tentative"),
        ];
        let status_idx = match self.config.event_status_filter {
            EventStatusFilter::All => Some(0),
            EventStatusFilter::Accepted => Some(1),
            EventStatusFilter::AcceptedOrTentative => Some(2),
        };

        // Email summary for navigation link
        let email_count = self.config.additional_emails.len();
        let email_summary = fl!("additional-emails-summary", count = email_count);

        // Filter settings group
        let mut filter_settings = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Show all-day events toggle
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("show-all-day-events")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::toggler(self.config.show_all_day_events)
                            .on_toggle(Message::SetShowAllDayEvents),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            )
            // Status filter dropdown
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("status-filter-section")))
                    .push(widget::horizontal_space())
                    .push(widget::dropdown(
                        status_options,
                        status_idx,
                        Message::SetEventStatusFilter,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );

        // Show email settings link only when filtering by status
        if self.config.event_status_filter != EventStatusFilter::All {
            filter_settings = filter_settings.add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("additional-emails-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(email_summary))
                                .push(
                                    widget::icon::from_name("go-next-symbolic").size(space.space_m),
                                )
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center),
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::EmailSettings)),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );
        }

        content = content.push(filter_settings);

        // Description at bottom with secondary color
        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
        content = content.push(widget::vertical_space().height(space.space_s));
        content = content
            .push(widget::text::caption(fl!("filter-events-description")).class(secondary_text));
        content = content.push(widget::vertical_space().height(space.space_m));

        content.into()
    }

    /// Email settings page for additional email addresses
    fn view_email_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(6 + self.config.additional_emails.len())
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .width(Length::Fill);

        // Back button header
        content = content.push(
            widget::column::with_capacity(2)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("filter-events-section"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::EventsToShowSettings)),
                )
                .push(widget::text::title4(fl!("additional-emails-section")))
                .spacing(space.space_xxs),
        );

        // Email list as a grouped list
        let mut emails_list =
            widget::list_column().list_item_padding([space.space_xxs, space.space_xs]);

        for (idx, email) in self.config.additional_emails.iter().enumerate() {
            emails_list = emails_list.add(
                widget::row::with_capacity(2)
                    .push(
                        widget::text_input("email@example.com", email)
                            .on_input(move |s| Message::UpdateEmail(idx, s))
                            .width(Length::Fill)
                            .id(email_input_id(idx)),
                    )
                    .push(
                        widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
                            .extra_small()
                            .on_press(Message::RemoveEmail(idx)),
                    )
                    .spacing(space.space_xs)
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );
        }

        content = content.push(emails_list);

        // Add email button
        content =
            content.push(widget::button::standard(fl!("add-email")).on_press(Message::AddEmail));

        // Description at bottom with secondary color
        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
        content = content.push(widget::vertical_space().height(space.space_s));
        content = content.push(
            widget::text::caption(fl!("additional-emails-description")).class(secondary_text),
        );
        content = content.push(widget::vertical_space().height(space.space_m));

        content.into()
    }

    /// Refresh settings page
    fn view_refresh_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(6)
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .width(Length::Fill);

        // Back button header
        content = content.push(
            widget::column::with_capacity(2)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("settings"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Settings)),
                )
                .push(widget::text::title4(fl!("refresh-section")))
                .spacing(space.space_xxs),
        );

        // Interval dropdown options
        let interval_options = vec![
            fl!("refresh-interval-5m"),
            fl!("refresh-interval-10m"),
            fl!("refresh-interval-15m"),
            fl!("refresh-interval-30m"),
        ];

        // Current interval index
        let interval_idx = match self.config.auto_refresh_interval_minutes {
            5 => Some(0),
            10 => Some(1),
            15 => Some(2),
            30 => Some(3),
            _ => Some(1),
        };

        // Refresh settings group
        let mut refresh_settings = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Auto-refresh toggle
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("auto-refresh")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::toggler(self.config.auto_refresh_enabled)
                            .on_toggle(Message::SetAutoRefresh),
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );

        // Only show interval dropdown when auto-refresh is enabled
        if self.config.auto_refresh_enabled {
            refresh_settings = refresh_settings.add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("refresh-interval")))
                    .push(widget::horizontal_space())
                    .push(widget::dropdown(
                        interval_options,
                        interval_idx,
                        Message::SetAutoRefreshInterval,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill),
            );
        }

        content = content.push(refresh_settings);

        // Sync manually button
        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
        content = content.push(widget::vertical_space().height(space.space_xs));

        let sync_button = widget::button::icon(widget::icon::from_name("view-refresh-symbolic"))
            .label(if self.is_refreshing {
                fl!("refreshing")
            } else {
                fl!("refresh-now")
            });
        let sync_button = if self.is_refreshing {
            sync_button
        } else {
            sync_button.on_press(Message::RefreshCalendars)
        };

        content = content.push(sync_button);

        // Description at bottom with secondary color
        content = content.push(widget::vertical_space().height(space.space_s));
        content =
            content.push(widget::text::caption(fl!("refresh-description")).class(secondary_text));
        content = content.push(widget::vertical_space().height(space.space_m));

        content.into()
    }

    /// About page with app info
    fn view_about_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);

        let mut content = widget::column::with_capacity(6)
            .padding(space.space_xs)
            .spacing(space.space_xs)
            .align_x(cosmic::iced::Alignment::Center)
            .width(Length::Fill);

        // Back button header (left-aligned)
        content = content.push(
            widget::column::with_capacity(1)
                .push(
                    widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                        .extra_small()
                        .padding(space.space_none)
                        .label(fl!("settings"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Settings)),
                )
                .width(Length::Fill),
        );

        // Vertical space before icon
        content = content.push(widget::vertical_space().height(space.space_m));

        // App icon (centered, large)
        content = content.push(widget::icon::from_name("com.dangrover.next-meeting-app").size(64));

        // App name
        content = content.push(widget::text::title3(fl!("app-title")));

        // Version
        let version = env!("CARGO_PKG_VERSION");
        content = content
            .push(widget::text::body(fl!("version", version = version)).class(secondary_text));

        // Author
        let author = env!("CARGO_PKG_AUTHORS");
        content =
            content.push(widget::text::body(fl!("author", author = author)).class(secondary_text));

        // Vertical space at bottom
        content = content.push(widget::vertical_space().height(space.space_l));

        content.into()
    }
}

/// Create a calendar color indicator dot widget with optional tooltip showing calendar name
fn calendar_color_dot<'a, M: 'a>(
    calendar_uid: &str,
    calendars: &[CalendarInfo],
    size: f32,
    tooltip_position: Option<widget::tooltip::Position>,
) -> Option<Element<'a, M>> {
    // Find the calendar by UID and get its color
    let calendar = calendars.iter().find(|c| c.uid == calendar_uid)?;
    let color_hex = calendar.color.as_ref()?;
    let color = parse_hex_color(color_hex)?;
    let calendar_name = calendar.display_name.clone();

    let dot = widget::container(widget::Space::new(0, 0))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .class(cosmic::theme::Container::custom(move |_theme| {
            cosmic::iced_widget::container::Style {
                background: Some(cosmic::iced::Background::Color(color)),
                border: cosmic::iced::Border {
                    radius: (size / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        }));

    Some(match tooltip_position {
        Some(pos) => widget::tooltip(dot, widget::text(calendar_name), pos).into(),
        None => dot.into(),
    })
}

/// Open an event in the user's default calendar application.
/// For GNOME Calendar, uses --uuid to open the specific event.
/// For other apps, just opens the calendar application.
fn open_event_in_calendar(event_uid: &str) {
    // Query the default calendar application
    let desktop_file = std::process::Command::new("xdg-mime")
        .args(["query", "default", "text/calendar"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if desktop_file.is_empty() {
        return;
    }

    // Check if it's GNOME Calendar (supports --uuid flag)
    if desktop_file.contains("gnome-calendar") {
        let _ = std::process::Command::new("gnome-calendar")
            .arg("--uuid")
            .arg(event_uid)
            .spawn();
    } else {
        // For other calendar apps, just open the app
        let _ = std::process::Command::new("gtk-launch")
            .arg(&desktop_file)
            .spawn();
    }
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    UpdateConfig(Config),
    MeetingsUpdated(Vec<Meeting>),
    CalendarsLoaded(Vec<CalendarInfo>),
    ToggleCalendar(String),
    SelectDisplayFormat(usize),
    SetUpcomingEventsCount(i32),
    Navigate(PopupPage),
    OpenCalendar,
    OpenEvent(String),
    OpenMeetingUrl(String),
    SetPopupJoinButton(usize),
    SetPanelJoinButton(usize),
    SetPopupLocation(usize),
    SetPanelLocation(usize),
    SetPanelCalendarIndicator(bool),
    SetPopupCalendarIndicator(bool),
    UpdatePattern(usize, String),
    AddPattern,
    RemovePattern(usize),
    SetShowAllDayEvents(bool),
    SetEventStatusFilter(usize),
    UpdateEmail(usize, String),
    AddEmail,
    RemoveEmail(usize),
    RefreshCalendars,
    RefreshCompleted,
    SetAutoRefresh(bool),
    SetAutoRefreshInterval(usize),
    CalendarChanged,
}

/// Create a COSMIC application from the app model
impl cosmic::Application for AppModel {
    /// The async executor that will be used to run your application's commands.
    type Executor = cosmic::executor::Default;

    /// Data that your application receives to its init method.
    type Flags = ();

    /// Messages which the application and its widgets will emit.
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "com.dangrover.next-meeting-app";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        // Load configuration
        let config_context = cosmic_config::Config::new(Self::APP_ID, Config::VERSION).ok();
        let config = config_context
            .as_ref()
            .map(|ctx| Config::get_entry(ctx).unwrap_or_else(|(_e, c)| c))
            .unwrap_or_default();

        let enabled_uids = config.enabled_calendar_uids.clone();
        let upcoming_count = config.upcoming_events_count as usize;
        let additional_emails = config.additional_emails.clone();

        // Construct the app model with the runtime's core.
        let app = AppModel {
            core,
            config,
            config_context,
            ..Default::default()
        };

        // Fetch initial calendar list and meeting data
        let calendars_task = Task::perform(
            async { crate::calendar::get_available_calendars().await },
            |calendars| Message::CalendarsLoaded(calendars).into(),
        );

        let meetings_task = Task::perform(
            async move {
                crate::calendar::get_upcoming_meetings(
                    &enabled_uids,
                    upcoming_count + 1,
                    &additional_emails,
                )
                .await
            },
            |meetings| Message::MeetingsUpdated(meetings).into(),
        );

        (app, Task::batch([calendars_task, meetings_task]))
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// Describes the interface based on the current state of the application model.
    ///
    /// The applet's button in the panel will be drawn using the main view method.
    /// This view should emit messages to toggle the applet's popup window, which will
    /// be drawn using the `view_window` method.
    fn view(&self) -> Element<'_, Self::Message> {
        use chrono::Local;
        let space = spacing();
        let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);

        // Build panel content based on whether we have meetings
        let filtered = self.filtered_meetings();
        let (panel_content, show_panel_join) = if let Some(meeting) = filtered.first() {
            // Truncate title if needed
            let title = if meeting.title.len() > 30 {
                format!("{}...", &meeting.title[..27])
            } else {
                meeting.title.clone()
            };

            let now = Local::now();
            let minutes_until = meeting.start.signed_duration_since(now).num_minutes();

            // Check if we should show location based on visibility settings
            let is_same_day = meeting.start.date_naive() == now.date_naive();
            let show_location = match self.config.panel_location {
                LocationVisibility::Hide => false,
                LocationVisibility::Show => true,
                LocationVisibility::ShowIfSameDay => is_same_day,
                LocationVisibility::ShowIf30m => minutes_until <= 30,
                LocationVisibility::ShowIf15m => minutes_until <= 15,
                LocationVisibility::ShowIf5m => minutes_until <= 5,
            };

            let physical_location = if show_location {
                get_physical_location(meeting, &self.config.meeting_url_patterns)
            } else {
                None
            };

            // Get time string based on display format (smart formatting for date mode)
            let time_str = match self.config.display_format {
                DisplayFormat::Relative => {
                    let duration = meeting.start.signed_duration_since(now);
                    format_relative_time(duration)
                }
                _ => {
                    // Smart date formatting: just time if today, day+time if different day
                    format_panel_time(&meeting.start, &now)
                }
            };

            // Build the parenthetical string: "(time in Location)" or just "(time)"
            let info_str = match physical_location {
                Some(loc) => format!(
                    "  {}",
                    fl!(
                        "panel-time-location",
                        time = time_str.clone(),
                        location = loc
                    )
                ),
                None => format!("  {}", fl!("panel-time", time = time_str)),
            };

            // Create styled text with optional calendar indicator: "[dot] Title (time in Location)"
            let mut content = widget::row::with_capacity(3)
                .spacing(space.space_xxs)
                .align_y(cosmic::iced::Alignment::Center);

            // Add calendar indicator dot if enabled
            if self.config.panel_calendar_indicator
                && let Some(dot) = calendar_color_dot::<Message>(
                    &meeting.calendar_uid,
                    &self.available_calendars,
                    8.0,
                    Some(widget::tooltip::Position::Bottom),
                )
            {
                content = content.push(dot);
            }

            content = content
                .push(self.core.applet.text(title).font(cosmic::iced::font::Font {
                    weight: cosmic::iced::font::Weight::Bold,
                    ..cosmic::iced::font::Font::DEFAULT
                }))
                .push(self.core.applet.text(info_str).class(secondary_text));
            let join_url = match self.config.panel_join_button {
                JoinButtonVisibility::Hide => None,
                JoinButtonVisibility::Show => {
                    extract_meeting_url(meeting, &self.config.meeting_url_patterns)
                }
                JoinButtonVisibility::ShowIfSameDay if is_same_day => {
                    extract_meeting_url(meeting, &self.config.meeting_url_patterns)
                }
                JoinButtonVisibility::ShowIf30m if minutes_until <= 30 => {
                    extract_meeting_url(meeting, &self.config.meeting_url_patterns)
                }
                JoinButtonVisibility::ShowIf15m if minutes_until <= 15 => {
                    extract_meeting_url(meeting, &self.config.meeting_url_patterns)
                }
                JoinButtonVisibility::ShowIf5m if minutes_until <= 5 => {
                    extract_meeting_url(meeting, &self.config.meeting_url_patterns)
                }
                _ => None,
            };

            (content, join_url)
        } else if self.available_calendars.is_empty() {
            // No calendars configured - show warning
            let content = widget::row::with_capacity(2)
                .spacing(space.space_xxs)
                .align_y(cosmic::iced::Alignment::Center)
                .push(widget::icon::from_name("dialog-warning-symbolic").size(space.space_m))
                .push(self.core.applet.text(fl!("no-calendars")));
            (content, None)
        } else {
            let content =
                widget::row::with_capacity(1).push(self.core.applet.text(fl!("no-meetings-panel")));
            (content, None)
        };

        // Main panel button with meeting text
        let main_button =
            widget::button::custom(panel_content.padding([space.space_xxxs, space.space_xs]))
                .class(cosmic::theme::Button::AppletIcon)
                .on_press(Message::TogglePopup);

        let mut row = widget::row::with_capacity(2)
            .push(main_button)
            .align_y(cosmic::iced::Alignment::Center)
            .spacing(space.space_xxs);

        // Add join button next to panel button if we should show it
        if let Some(url) = show_panel_join {
            // Use space_xxs (8) + space_xxxs (4) = 12px for compact panel text
            let font_size = space.space_xxs + space.space_xxxs;
            row = row.push(
                widget::button::custom(
                    widget::text(fl!("join"))
                        .size(font_size)
                        .font(cosmic::iced::font::Font {
                            weight: cosmic::iced::font::Weight::Bold,
                            ..cosmic::iced::font::Font::DEFAULT
                        })
                        .line_height(cosmic::iced::widget::text::LineHeight::Absolute(
                            font_size.into(),
                        )),
                )
                .padding([space.space_xxxs, space.space_xxs])
                .class(cosmic::theme::Button::Suggested)
                .on_press(Message::OpenMeetingUrl(url)),
            );
        }

        self.core.applet.autosize_window(row).into()
    }

    /// The applet's popup window will be drawn using this view method. If there are
    /// multiple poups, you may match the id parameter to determine which popup to
    /// create a view for.
    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let content: Element<'_, Self::Message> = match self.current_page {
            PopupPage::Main => self.view_main_page(),
            PopupPage::Settings => self.view_settings_page(),
            PopupPage::Calendars => self.view_calendars_page(),
            PopupPage::RefreshSettings => self.view_refresh_settings_page(),
            PopupPage::JoinButtonSettings => self.view_join_button_settings_page(),
            PopupPage::LocationSettings => self.view_location_settings_page(),
            PopupPage::CalendarIndicatorSettings => self.view_calendar_indicator_settings_page(),
            PopupPage::EventsToShowSettings => self.view_events_to_show_settings_page(),
            PopupPage::EmailSettings => self.view_email_settings_page(),
            PopupPage::About => self.view_about_page(),
        };

        // Consistent popup size limits
        let limits = Limits::NONE
            .max_width(360.0)
            .min_width(360.0)
            .min_height(200.0)
            .max_height(800.0);

        self.core
            .applet
            .popup_container(content)
            .limits(limits)
            .into()
    }

    /// Register subscriptions for this application.
    ///
    /// Subscriptions are long-lived async tasks running in the background which
    /// emit messages to the application through a channel. They may be conditionally
    /// activated by selectively appending to the subscription batch, and will
    /// continue to execute for the duration that they remain in the batch.
    fn subscription(&self) -> Subscription<Self::Message> {
        let enabled_uids = self.config.enabled_calendar_uids.clone();
        let upcoming_count = self.config.upcoming_events_count as usize;
        let additional_emails = self.config.additional_emails.clone();
        let auto_refresh_enabled = self.config.auto_refresh_enabled;
        let auto_refresh_interval = self.config.auto_refresh_interval_minutes;

        // Create a unique subscription ID based on config values that affect filtering.
        // When these change, the subscription will be recreated with the new values.
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        enabled_uids.hash(&mut hasher);
        upcoming_count.hash(&mut hasher);
        additional_emails.hash(&mut hasher);
        let config_hash = hasher.finish();

        // Create a separate hash for auto-refresh subscription
        let mut refresh_hasher = std::collections::hash_map::DefaultHasher::new();
        auto_refresh_enabled.hash(&mut refresh_hasher);
        auto_refresh_interval.hash(&mut refresh_hasher);
        enabled_uids.hash(&mut refresh_hasher);
        let refresh_hash = refresh_hasher.finish();

        let mut subscriptions = vec![
            // Periodically read cached calendar and meeting data (every 60 seconds)
            Subscription::run_with_id(
                config_hash,
                cosmic::iced::stream::channel(4, move |mut channel| async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                    loop {
                        interval.tick().await;
                        // Read cached calendars and meetings
                        let calendars = crate::calendar::get_available_calendars().await;
                        let _ = channel.send(Message::CalendarsLoaded(calendars)).await;
                        let meetings = crate::calendar::get_upcoming_meetings(
                            &enabled_uids,
                            upcoming_count + 1,
                            &additional_emails,
                        )
                        .await;
                        let _ = channel.send(Message::MeetingsUpdated(meetings)).await;
                    }
                }),
            ),
            // Watch for application configuration changes.
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ];

        // Add auto-refresh subscription if enabled
        if auto_refresh_enabled {
            let refresh_uids = self.config.enabled_calendar_uids.clone();
            subscriptions.push(Subscription::run_with_id(
                refresh_hash,
                cosmic::iced::stream::channel(2, move |mut channel| async move {
                    let interval_secs = u64::from(auto_refresh_interval) * 60;
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                    // Skip the first immediate tick
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        // Trigger a refresh from remote servers
                        crate::calendar::refresh_calendars(&refresh_uids).await;
                        // Signal that refresh started (the 60-second subscription will pick up new data)
                        let _ = channel.send(Message::RefreshCalendars).await;
                    }
                }),
            ));
        }

        // Watch for D-Bus PropertiesChanged signals from EDS calendars
        // This detects when calendars are updated after a sync (by us or external apps)
        let watch_uids = self.config.enabled_calendar_uids.clone();
        subscriptions.push(Subscription::run_with_id(
            ("calendar-changes", config_hash),
            cosmic::iced::stream::channel(4, move |mut channel| async move {
                let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(4);

                // Spawn the watcher in a separate task
                let watch_task =
                    tokio::spawn(crate::calendar::watch_calendar_changes(watch_uids, sender));

                // Forward messages from the watcher to the iced channel
                while receiver.recv().await.is_some() {
                    let _ = channel.send(Message::CalendarChanged).await;
                }

                // Clean up if the watcher exits
                watch_task.abort();
            }),
        ));

        Subscription::batch(subscriptions)
    }

    /// Handles messages emitted by the application and its widgets.
    ///
    /// Tasks may be returned for asynchronous execution of code in the background
    /// on the application's async runtime. The application will not exit until all
    /// tasks are finished.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::UpdateConfig(config) => {
                self.config = config;
            }
            Message::MeetingsUpdated(meetings) => {
                self.upcoming_meetings = meetings;
            }
            Message::CalendarsLoaded(calendars) => {
                self.available_calendars = calendars;
            }
            Message::ToggleCalendar(uid) => {
                // If the list is empty (all enabled), populate it with all meeting-source calendars
                if self.config.enabled_calendar_uids.is_empty() {
                    self.config.enabled_calendar_uids = self
                        .available_calendars
                        .iter()
                        .filter(|c| c.is_meeting_source())
                        .map(|c| c.uid.clone())
                        .collect();
                }

                // Toggle the calendar
                if self.config.enabled_calendar_uids.contains(&uid) {
                    self.config.enabled_calendar_uids.retain(|u| u != &uid);
                } else {
                    self.config.enabled_calendar_uids.push(uid);
                }

                // Save config
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }

                // Refresh meetings with new filter
                let enabled_uids = self.enabled_meeting_source_uids();
                let upcoming_count = self.config.upcoming_events_count as usize;
                let additional_emails = self.config.additional_emails.clone();
                return Task::perform(
                    async move {
                        crate::calendar::get_upcoming_meetings(
                            &enabled_uids,
                            upcoming_count + 1,
                            &additional_emails,
                        )
                        .await
                    },
                    |meetings| Message::MeetingsUpdated(meetings).into(),
                );
            }
            Message::SelectDisplayFormat(idx) => {
                self.config.display_format = match idx {
                    0 => DisplayFormat::DayAndTime,
                    1 => DisplayFormat::Relative,
                    _ => DisplayFormat::DayAndTime,
                };
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::SetUpcomingEventsCount(count) => {
                self.config.upcoming_events_count = count.clamp(0, 10) as u8;
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
                // Refresh meetings with new count
                let enabled_uids = self.config.enabled_calendar_uids.clone();
                let upcoming_count = self.config.upcoming_events_count as usize;
                let additional_emails = self.config.additional_emails.clone();
                return Task::perform(
                    async move {
                        crate::calendar::get_upcoming_meetings(
                            &enabled_uids,
                            upcoming_count + 1,
                            &additional_emails,
                        )
                        .await
                    },
                    |meetings| Message::MeetingsUpdated(meetings).into(),
                );
            }
            Message::Navigate(page) => {
                self.current_page = page;
            }
            Message::OpenCalendar => {
                // Query the default calendar application and launch it using gtk-launch
                if let Ok(output) = std::process::Command::new("xdg-mime")
                    .args(["query", "default", "text/calendar"])
                    .output()
                {
                    let desktop_file = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !desktop_file.is_empty() {
                        // gtk-launch works with just the desktop file name (without .desktop suffix too)
                        let _ = std::process::Command::new("gtk-launch")
                            .arg(&desktop_file)
                            .spawn();
                    }
                }
            }
            Message::OpenEvent(uid) => {
                open_event_in_calendar(&uid);
            }
            Message::OpenMeetingUrl(url) => {
                // Open the meeting URL in the default browser
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
            }
            Message::SetPopupJoinButton(idx) => {
                self.config.popup_join_button = match idx {
                    0 => JoinButtonVisibility::Hide,
                    1 => JoinButtonVisibility::Show,
                    2 => JoinButtonVisibility::ShowIfSameDay,
                    3 => JoinButtonVisibility::ShowIf30m,
                    4 => JoinButtonVisibility::ShowIf15m,
                    5 => JoinButtonVisibility::ShowIf5m,
                    _ => JoinButtonVisibility::Show,
                };
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::SetPanelJoinButton(idx) => {
                self.config.panel_join_button = match idx {
                    0 => JoinButtonVisibility::Hide,
                    1 => JoinButtonVisibility::Show,
                    2 => JoinButtonVisibility::ShowIfSameDay,
                    3 => JoinButtonVisibility::ShowIf30m,
                    4 => JoinButtonVisibility::ShowIf15m,
                    5 => JoinButtonVisibility::ShowIf5m,
                    _ => JoinButtonVisibility::Show,
                };
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::SetPopupLocation(idx) => {
                self.config.popup_location = match idx {
                    0 => LocationVisibility::Hide,
                    1 => LocationVisibility::Show,
                    2 => LocationVisibility::ShowIfSameDay,
                    3 => LocationVisibility::ShowIf30m,
                    4 => LocationVisibility::ShowIf15m,
                    5 => LocationVisibility::ShowIf5m,
                    _ => LocationVisibility::Show,
                };
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::SetPanelLocation(idx) => {
                self.config.panel_location = match idx {
                    0 => LocationVisibility::Hide,
                    1 => LocationVisibility::Show,
                    2 => LocationVisibility::ShowIfSameDay,
                    3 => LocationVisibility::ShowIf30m,
                    4 => LocationVisibility::ShowIf15m,
                    5 => LocationVisibility::ShowIf5m,
                    _ => LocationVisibility::Show,
                };
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::SetPanelCalendarIndicator(enabled) => {
                self.config.panel_calendar_indicator = enabled;
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::SetPopupCalendarIndicator(enabled) => {
                self.config.popup_calendar_indicator = enabled;
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::UpdatePattern(idx, pattern) => {
                if idx < self.config.meeting_url_patterns.len() {
                    self.config.meeting_url_patterns[idx] = pattern;
                    if let Some(ref ctx) = self.config_context {
                        let _ = self.config.write_entry(ctx);
                    }
                }
            }
            Message::AddPattern => {
                self.config.meeting_url_patterns.push(String::new());
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::RemovePattern(idx) => {
                if idx < self.config.meeting_url_patterns.len() {
                    self.config.meeting_url_patterns.remove(idx);
                    if let Some(ref ctx) = self.config_context {
                        let _ = self.config.write_entry(ctx);
                    }
                }
            }
            Message::SetShowAllDayEvents(enabled) => {
                self.config.show_all_day_events = enabled;
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::SetEventStatusFilter(idx) => {
                use crate::config::EventStatusFilter;
                self.config.event_status_filter = match idx {
                    0 => EventStatusFilter::All,
                    1 => EventStatusFilter::Accepted,
                    2 => EventStatusFilter::AcceptedOrTentative,
                    _ => EventStatusFilter::All,
                };
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::UpdateEmail(idx, email) => {
                if let Some(e) = self.config.additional_emails.get_mut(idx) {
                    *e = email;
                }
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::AddEmail => {
                let new_idx = self.config.additional_emails.len();
                self.config.additional_emails.push(String::new());
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
                // Focus the new email input field
                return cosmic::widget::text_input::focus(email_input_id(new_idx));
            }
            Message::RemoveEmail(idx) => {
                if idx < self.config.additional_emails.len() {
                    self.config.additional_emails.remove(idx);
                }
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::RefreshCalendars => {
                if !self.is_refreshing {
                    self.is_refreshing = true;
                    let enabled_uids = self.enabled_meeting_source_uids();
                    let upcoming_count = self.config.upcoming_events_count as usize;
                    let additional_emails = self.config.additional_emails.clone();
                    return Task::perform(
                        async move {
                            // First refresh calendars from remote servers
                            crate::calendar::refresh_calendars(&enabled_uids).await;
                            // Wait a moment for EDS to process the refresh
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            // Then fetch updated meetings
                            crate::calendar::get_upcoming_meetings(
                                &enabled_uids,
                                upcoming_count + 1,
                                &additional_emails,
                            )
                            .await
                        },
                        |meetings| Message::MeetingsUpdated(meetings).into(),
                    )
                    .chain(Task::done(Message::RefreshCompleted.into()));
                }
            }
            Message::RefreshCompleted => {
                self.is_refreshing = false;
            }
            Message::SetAutoRefresh(enabled) => {
                self.config.auto_refresh_enabled = enabled;
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::SetAutoRefreshInterval(idx) => {
                self.config.auto_refresh_interval_minutes = match idx {
                    0 => 5,
                    1 => 10,
                    2 => 15,
                    3 => 30,
                    _ => 10,
                };
                if let Some(ref ctx) = self.config_context {
                    let _ = self.config.write_entry(ctx);
                }
            }
            Message::CalendarChanged => {
                // A calendar was updated via D-Bus signal (sync completed)
                // Refresh both calendars list (for updated sync timestamps) and meetings
                let enabled_uids = self.enabled_meeting_source_uids();
                let upcoming_count = self.config.upcoming_events_count as usize;
                let additional_emails = self.config.additional_emails.clone();

                let calendars_task = Task::perform(
                    async { crate::calendar::get_available_calendars().await },
                    |calendars| Message::CalendarsLoaded(calendars).into(),
                );

                let meetings_task = Task::perform(
                    async move {
                        crate::calendar::get_upcoming_meetings(
                            &enabled_uids,
                            upcoming_count + 1,
                            &additional_emails,
                        )
                        .await
                    },
                    |meetings| Message::MeetingsUpdated(meetings).into(),
                );

                return Task::batch([calendars_task, meetings_task]);
            }
            Message::TogglePopup => {
                return if let Some(p) = self.popup.take() {
                    destroy_popup(p)
                } else {
                    let new_id = Id::unique();
                    self.popup.replace(new_id);
                    let mut popup_settings = self.core.applet.get_popup_settings(
                        self.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    );
                    popup_settings.positioner.size_limits = Limits::NONE;
                    get_popup(popup_settings)
                };
            }
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced_runtime::Appearance> {
        Some(cosmic::applet::style())
    }
}
