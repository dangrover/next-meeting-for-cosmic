// SPDX-License-Identifier: MPL-2.0

use crate::calendar::{CalendarInfo, Meeting};
use crate::config::{Config, DisplayFormat, JoinButtonVisibility, LocationVisibility};
use crate::fl;
use cosmic::cosmic_config::{self, ConfigGet, CosmicConfigEntry};
use cosmic::cosmic_theme;
use cosmic::iced::{window::Id, Length, Limits, Subscription};
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


/// Check if user prefers 24-hour (military) time from COSMIC settings
fn use_military_time() -> bool {
    cosmic::cosmic_config::Config::new("com.system76.CosmicAppletTime", 1)
        .ok()
        .and_then(|config| config.get::<bool>("military_time").ok())
        .unwrap_or(false)
}

/// Format a time according to user's COSMIC time preference
fn format_time(dt: &chrono::DateTime<chrono::Local>, include_day: bool) -> String {
    let time_fmt = if use_military_time() { "%H:%M" } else { "%I:%M %p" };
    if include_day {
        dt.format(&format!("%A, %B %d at {}", time_fmt)).to_string()
    } else {
        dt.format(&format!("%a {}", time_fmt)).to_string()
    }
}

/// Smart panel time formatting: just time if today, day+time if different day
fn format_panel_time(dt: &chrono::DateTime<chrono::Local>, now: &chrono::DateTime<chrono::Local>) -> String {
    let time_fmt = if use_military_time() { "%H:%M" } else { "%l:%M%P" }; // %l = hour 1-12 no padding, %P = lowercase am/pm
    let is_same_day = dt.date_naive() == now.date_naive();

    if is_same_day {
        // Just show time: "2:30pm" or "14:30"
        dt.format(time_fmt).to_string().trim().to_string()
    } else {
        // Show day and time: "Fri 2:30pm" or "Fri 14:30"
        dt.format(&format!("%a {}", time_fmt)).to_string().trim().to_string()
    }
}

/// Get theme spacing values
fn spacing() -> cosmic_theme::Spacing {
    cosmic::theme::spacing()
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
                background: Some(cosmic::iced::Background::Color(cosmic.text_button.hover.into())),
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
                background: Some(cosmic::iced::Background::Color(cosmic.text_button.pressed.into())),
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
}

/// Navigation state for popup pages
#[derive(Debug, Default, Clone, PartialEq)]
pub enum PopupPage {
    #[default]
    Main,
    Settings,
    Calendars,
    JoinButtonSettings,
    LocationSettings,
    CalendarIndicatorSettings,
}

impl AppModel {
    /// Format meeting text based on current display format setting
    fn format_meeting_text(&self, meeting: &Meeting) -> String {
        use chrono::Local;

        let title = if meeting.title.len() > 40 {
            format!("{}...", &meeting.title[..37])
        } else {
            meeting.title.clone()
        };

        match self.config.display_format {
            DisplayFormat::Relative => {
                let now = Local::now();
                let duration = meeting.start.signed_duration_since(now);
                let relative = format_relative_time(duration);
                format!("{}: {}", relative, title)
            }
            _ => {
                // DayAndTime is the default
                let time_str = format_time(&meeting.start, false);
                format!("{}: {}", time_str, title)
            }
        }
    }

    /// Main popup page showing meeting info and settings nav
    fn view_main_page(&self) -> Element<'_, Message> {
        let space = spacing();

        // Match Power applet pattern: column with [8, 0] padding (vertical only)
        let mut content = widget::column::with_capacity(8)
            .padding([8, 0]);

        if let Some(meeting) = self.upcoming_meetings.first() {
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
                if let Some(dot) = calendar_color_dot::<Message>(&meeting.calendar_uid, &self.available_calendars, 10.0, Some(widget::tooltip::Position::Top)) {
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
                meeting_column = meeting_column.push(widget::text::body(location).class(secondary_text));
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
                                .on_press(Message::OpenMeetingUrl(url))
                        )
                        .align_y(cosmic::iced::Alignment::Center)
                        .spacing(space.space_xs)
                        .width(Length::Fill)
                        .apply(widget::container)
                        .padding([0, space.space_s])
                );
            } else {
                // Wrap in container with horizontal padding
                content = content.push(
                    meeting_info
                        .apply(widget::container)
                        .padding([0, space.space_s])
                );
            }

            // Upcoming events section
            let upcoming_count = self.config.upcoming_events_count as usize;
            if upcoming_count > 0 && self.upcoming_meetings.len() > 1 {
                // Divider before "Upcoming" section (matching Power applet pattern)
                content = content.push(
                    cosmic::applet::padded_control(widget::divider::horizontal::default())
                        .padding([space.space_xxs, space.space_s])
                );

                // "Upcoming" section heading
                content = content.push(
                    cosmic::applet::padded_control(widget::text::heading(fl!("upcoming")))
                );

                let secondary_text = cosmic::theme::Text::Custom(secondary_text_style);
                for meeting in self.upcoming_meetings.iter().skip(1).take(upcoming_count) {
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

                    if self.config.popup_calendar_indicator {
                        if let Some(dot) = calendar_color_dot::<Message>(&meeting.calendar_uid, &self.available_calendars, 8.0, None) {
                            row = row.push(dot);
                        }
                    }

                    row = row
                        .push(widget::text::body(title))
                        .push(widget::horizontal_space())
                        .push(widget::text::body(time_str).class(secondary_text));

                    content = content.push(
                        cosmic::applet::menu_button(row)
                            .on_press(Message::OpenEvent(uid))
                    );
                }
            }
        } else {
            content = content.push(
                cosmic::applet::padded_control(widget::text::body(fl!("no-meetings")))
            );
        }

        // Divider before bottom actions (matching Power applet pattern)
        content = content.push(
            cosmic::applet::padded_control(widget::divider::horizontal::default())
                .padding([space.space_xxs, space.space_s])
        );

        // Bottom actions section (Open calendar + Settings)
        content = content.push(
            cosmic::applet::menu_button(
                widget::row::with_capacity(3)
                    .push(widget::icon::from_name("office-calendar-symbolic").size(16))
                    .push(widget::text::body(fl!("open-calendar")))
                    .push(widget::horizontal_space())
                    .spacing(space.space_xs)
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
            )
            .on_press(Message::OpenCalendar)
        );

        content = content.push(
            cosmic::applet::menu_button(
                widget::row::with_capacity(3)
                    .push(widget::icon::from_name("preferences-system-symbolic").size(16))
                    .push(widget::text::body(fl!("settings")))
                    .push(widget::horizontal_space())
                    .spacing(space.space_xs)
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
            )
            .on_press(Message::Navigate(PopupPage::Settings))
        );

        content.into()
    }

    /// Settings page with back button
    fn view_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(2)
            .padding(space.space_xs)
            .spacing(space.space_xs);

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
                        .on_press(Message::Navigate(PopupPage::Main))
                )
                .push(widget::text::title4(fl!("settings")))
                .spacing(space.space_xxxs)
        );

        // Extra space after header
        content = content.push(widget::vertical_space().height(space.space_xxxs));

        // Calendars count for summary
        let enabled_count = if self.config.enabled_calendar_uids.is_empty() {
            self.available_calendars.len()
        } else {
            self.config.enabled_calendar_uids.len()
        };
        let calendar_summary = fl!("calendars-enabled", count = enabled_count);

        // Display format dropdown index
        let format_idx = match self.config.display_format {
            DisplayFormat::DayAndTime => Some(0),
            DisplayFormat::Relative => Some(1),
            _ => Some(0),
        };

        // Join button status summary
        let join_status = match (&self.config.panel_join_button, &self.config.popup_join_button) {
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
        let indicator_status = match (self.config.panel_calendar_indicator, self.config.popup_calendar_indicator) {
            (false, false) => fl!("status-off"),
            (false, true) => fl!("status-popup"),
            (true, false) => fl!("status-panel"),
            (true, true) => fl!("status-both"),
        };

        // Calendars section (its own group)
        let calendars_section = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("calendars-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(calendar_summary))
                                .push(widget::icon::from_name("go-next-symbolic").size(16))
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center)
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Calendars))
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
            );

        content = content.push(calendars_section);

        // More vertical spacing between sections
        content = content.push(widget::vertical_space().height(space.space_xs));

        // Other settings section
        let other_settings = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs])
            // Display format
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("display-format-section")))
                    .push(widget::horizontal_space())
                    .push(widget::dropdown(display_format_options(), format_idx, Message::SelectDisplayFormat))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
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
                    .width(Length::Fill)
            )
            // Join button settings
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("join-button-section")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::button::custom(
                            widget::row::with_capacity(2)
                                .push(widget::text::body(join_status))
                                .push(widget::icon::from_name("go-next-symbolic").size(16))
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center)
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::JoinButtonSettings))
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
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
                                .push(widget::icon::from_name("go-next-symbolic").size(16))
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center)
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::LocationSettings))
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
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
                                .push(widget::icon::from_name("go-next-symbolic").size(16))
                                .spacing(space.space_xxs)
                                .align_y(cosmic::iced::Alignment::Center)
                        )
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::CalendarIndicatorSettings))
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
            );

        content = content.push(other_settings);

        content.into()
    }

    /// Calendars selection page
    fn view_calendars_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(2 + self.available_calendars.len())
            .padding(space.space_xs)
            .spacing(space.space_xs);

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
                        .on_press(Message::Navigate(PopupPage::Settings))
                )
                .push(widget::text::title4(fl!("calendars-section")))
                .spacing(space.space_xxxs)
        );

        // Extra space after header
        content = content.push(widget::vertical_space().height(space.space_xxxs));

        // Calendar toggles in a single list_column
        let mut calendars_list = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs]);

        for calendar in &self.available_calendars {
            let is_enabled = self.config.enabled_calendar_uids.is_empty()
                || self.config.enabled_calendar_uids.contains(&calendar.uid);

            let uid = calendar.uid.clone();

            // Build row with optional color indicator
            let mut row = widget::row::with_capacity(4)
                .spacing(space.space_xs)
                .align_y(cosmic::iced::Alignment::Center);

            // Add color circle if color is available
            if let Some(color) = &calendar.color {
                if let Some(parsed_color) = parse_hex_color(color) {
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
                            }))
                    );
                }
            }

            row = row
                .push(widget::text::body(&calendar.display_name))
                .push(widget::horizontal_space())
                .push(widget::toggler(is_enabled)
                    .on_toggle(move |_| Message::ToggleCalendar(uid.clone())));

            calendars_list = calendars_list.add(row);
        }

        content = content.push(calendars_list);

        content.into()
    }

    /// Join button settings page
    fn view_join_button_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(6 + self.config.meeting_url_patterns.len())
            .padding(space.space_xs)
            .spacing(space.space_xs);

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
                        .on_press(Message::Navigate(PopupPage::Settings))
                )
                .push(widget::text::title4(fl!("join-button-section")))
                .spacing(space.space_xxxs)
        );

        // Extra space after header
        content = content.push(widget::vertical_space().height(space.space_xxxs));

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
                    .width(Length::Fill)
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
                    .width(Length::Fill)
            );

        content = content.push(visibility_settings);

        // Space before URL patterns section
        content = content.push(widget::vertical_space().height(space.space_xxs));

        // URL patterns section heading
        content = content.push(widget::text::heading(fl!("url-patterns")));

        // Pattern list as a grouped list
        let mut patterns_list = widget::list_column()
            .list_item_padding([space.space_xxs, space.space_xs]);

        for (idx, pattern) in self.config.meeting_url_patterns.iter().enumerate() {
            patterns_list = patterns_list.add(
                widget::row::with_capacity(2)
                    .push(
                        widget::text_input("", pattern)
                            .on_input(move |s| Message::UpdatePattern(idx, s))
                            .width(Length::Fill)
                    )
                    .push(
                        widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
                            .extra_small()
                            .on_press(Message::RemovePattern(idx))
                    )
                    .spacing(space.space_xs)
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
            );
        }

        content = content.push(patterns_list);

        // Add pattern button
        content = content.push(
            widget::button::standard(fl!("add-pattern"))
                .on_press(Message::AddPattern)
        );

        content.into()
    }

    /// Physical location settings page
    fn view_location_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(4)
            .padding(space.space_xs)
            .spacing(space.space_xs);

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
                        .on_press(Message::Navigate(PopupPage::Settings))
                )
                .push(widget::text::title4(fl!("location-section")))
                .spacing(space.space_xxxs)
        );

        // Extra space after header
        content = content.push(widget::vertical_space().height(space.space_xxxs));

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
                    .width(Length::Fill)
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
                    .width(Length::Fill)
            );

        content = content.push(visibility_settings);

        content.into()
    }

    /// Calendar indicator settings page
    fn view_calendar_indicator_settings_page(&self) -> Element<'_, Message> {
        let space = spacing();
        let mut content = widget::column::with_capacity(4)
            .padding(space.space_xs)
            .spacing(space.space_xs);

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
                        .on_press(Message::Navigate(PopupPage::Settings))
                )
                .push(widget::text::title4(fl!("calendar-indicator-section")))
                .spacing(space.space_xxxs)
        );

        // Extra space after header
        content = content.push(widget::vertical_space().height(space.space_xxxs));

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
                            .on_toggle(Message::SetPanelCalendarIndicator)
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
            )
            // Popup indicator toggle
            .add(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("popup-indicator")))
                    .push(widget::horizontal_space())
                    .push(
                        widget::toggler(self.config.popup_calendar_indicator)
                            .on_toggle(Message::SetPopupCalendarIndicator)
                    )
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
            );

        content = content.push(indicator_settings);

        content.into()
    }
}

/// Parse a hex color string (e.g., "#62a0ea") to an iced Color
fn parse_hex_color(hex: &str) -> Option<cosmic::iced::Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(cosmic::iced::Color::from_rgb8(r, g, b))
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

/// Format a duration as relative time (e.g., "in 2d 3h" or "in 2h 30m")
/// Shows minutes only when the event is within 24 hours
fn format_relative_time(duration: chrono::Duration) -> String {
    let total_minutes = duration.num_minutes();
    if total_minutes < 0 {
        return fl!("time-now");
    }

    let days = total_minutes / (24 * 60);
    let hours = (total_minutes % (24 * 60)) / 60;
    let minutes = total_minutes % 60;

    if days > 0 {
        // More than a day away - show days and hours, skip minutes
        if hours > 0 {
            fl!("time-in-days-hours", days = days, hours = hours)
        } else {
            fl!("time-in-days", days = days)
        }
    } else if hours > 0 {
        // Within 24 hours - show hours and minutes
        if minutes > 0 {
            fl!("time-in-hours-minutes", hours = hours, minutes = minutes)
        } else {
            fl!("time-in-hours", hours = hours)
        }
    } else {
        fl!("time-in-minutes", minutes = minutes)
    }
}

/// Extract a meeting URL from the meeting's location or description fields
/// using the provided regex patterns. Returns the first matching URL found.
fn extract_meeting_url(meeting: &Meeting, patterns: &[String]) -> Option<String> {
    // Compile patterns, skipping invalid ones
    let regexes: Vec<regex::Regex> = patterns
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect();

    if regexes.is_empty() {
        return None;
    }

    // Check location first (most common place for meeting links)
    if let Some(ref location) = meeting.location {
        for regex in &regexes {
            if let Some(m) = regex.find(location) {
                return Some(m.as_str().to_string());
            }
        }
    }

    // Then check description
    if let Some(ref description) = meeting.description {
        for regex in &regexes {
            if let Some(m) = regex.find(description) {
                return Some(m.as_str().to_string());
            }
        }
    }

    None
}

/// Get the physical location from a meeting (location that is not a URL).
/// Returns None if location is empty, matches a meeting URL pattern, or looks like a URL.
fn get_physical_location(meeting: &Meeting, patterns: &[String]) -> Option<String> {
    let location = meeting.location.as_ref()?;
    let location = location.trim();

    if location.is_empty() {
        return None;
    }

    // If location starts with http:// or https://, it's a URL
    if location.starts_with("http://") || location.starts_with("https://") {
        return None;
    }

    // Check if location matches any of the meeting URL patterns
    let regexes: Vec<regex::Regex> = patterns
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect();

    for regex in &regexes {
        if regex.is_match(location) {
            return None;
        }
    }

    Some(location.to_string())
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
            .map(|ctx| Config::get_entry(ctx).map_or_else(|(_e, c)| c, |c| c))
            .unwrap_or_default();

        let enabled_uids = config.enabled_calendar_uids.clone();
        let upcoming_count = config.upcoming_events_count as usize;

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
            async move { crate::calendar::get_upcoming_meetings(&enabled_uids, upcoming_count + 1).await },
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
        let (panel_content, show_panel_join) = if let Some(meeting) = self.upcoming_meetings.first() {
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
                Some(loc) => format!("  {}", fl!("panel-time-location", time = time_str.clone(), location = loc)),
                None => format!("  {}", fl!("panel-time", time = time_str)),
            };

            // Create styled text with optional calendar indicator: "[dot] Title (time in Location)"
            let mut content = widget::row::with_capacity(3)
                .spacing(space.space_xxs)
                .align_y(cosmic::iced::Alignment::Center);

            // Add calendar indicator dot if enabled
            if self.config.panel_calendar_indicator {
                if let Some(dot) = calendar_color_dot::<Message>(&meeting.calendar_uid, &self.available_calendars, 8.0, Some(widget::tooltip::Position::Bottom)) {
                    content = content.push(dot);
                }
            }

            content = content
                .push(
                    widget::text(title)
                        .font(cosmic::iced::font::Font {
                            weight: cosmic::iced::font::Weight::Bold,
                            ..cosmic::iced::font::Font::DEFAULT
                        })
                )
                .push(
                    widget::text(info_str)
                        .class(secondary_text)
                );
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
        } else {
            let content = widget::row::with_capacity(1)
                .push(widget::text(fl!("no-meetings-panel")));
            (content, None)
        };

        // Main panel button with meeting text
        let main_button = widget::button::custom(
            panel_content.padding([space.space_xxxs, space.space_xs])
        )
        .class(cosmic::theme::Button::AppletIcon)
        .on_press(Message::TogglePopup);

        let mut row = widget::row::with_capacity(2)
            .push(main_button)
            .align_y(cosmic::iced::Alignment::Center)
            .spacing(space.space_xxs);

        // Add join button next to panel button if we should show it
        if let Some(url) = show_panel_join {
            row = row.push(
                widget::button::custom(
                    widget::text::caption(fl!("join"))
                        .font(cosmic::iced::font::Font {
                            weight: cosmic::iced::font::Weight::Bold,
                            ..cosmic::iced::font::Font::DEFAULT
                        })
                )
                .padding([space.space_xxxs, space.space_xxs])
                .class(cosmic::theme::Button::Suggested)
                .on_press(Message::OpenMeetingUrl(url))
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
            PopupPage::JoinButtonSettings => self.view_join_button_settings_page(),
            PopupPage::LocationSettings => self.view_location_settings_page(),
            PopupPage::CalendarIndicatorSettings => self.view_calendar_indicator_settings_page(),
        };

        self.core.applet.popup_container(content).into()
    }

    /// Register subscriptions for this application.
    ///
    /// Subscriptions are long-lived async tasks running in the background which
    /// emit messages to the application through a channel. They may be conditionally
    /// activated by selectively appending to the subscription batch, and will
    /// continue to execute for the duration that they remain in the batch.
    fn subscription(&self) -> Subscription<Self::Message> {
        struct CalendarSubscription;

        let enabled_uids = self.config.enabled_calendar_uids.clone();
        let upcoming_count = self.config.upcoming_events_count as usize;

        Subscription::batch(vec![
            // Periodically refresh calendar and meeting data
            Subscription::run_with_id(
                std::any::TypeId::of::<CalendarSubscription>(),
                cosmic::iced::stream::channel(4, move |mut channel| async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                    loop {
                        interval.tick().await;
                        // Refresh both calendars and meetings
                        let calendars = crate::calendar::get_available_calendars().await;
                        let _ = channel.send(Message::CalendarsLoaded(calendars)).await;
                        let meetings = crate::calendar::get_upcoming_meetings(&enabled_uids, upcoming_count + 1).await;
                        let _ = channel.send(Message::MeetingsUpdated(meetings)).await;
                    }
                }),
            ),
            // Watch for application configuration changes.
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ])
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
                // If the list is empty (all enabled), populate it with all calendars first
                if self.config.enabled_calendar_uids.is_empty() {
                    self.config.enabled_calendar_uids = self
                        .available_calendars
                        .iter()
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
                let enabled_uids = self.config.enabled_calendar_uids.clone();
                let upcoming_count = self.config.upcoming_events_count as usize;
                return Task::perform(
                    async move { crate::calendar::get_upcoming_meetings(&enabled_uids, upcoming_count + 1).await },
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
                return Task::perform(
                    async move { crate::calendar::get_upcoming_meetings(&enabled_uids, upcoming_count + 1).await },
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
                let _ = std::process::Command::new("xdg-open")
                    .arg(&url)
                    .spawn();
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
                    popup_settings.positioner.size_limits = Limits::NONE
                        .max_width(372.0)
                        .min_width(300.0)
                        .min_height(200.0)
                        .max_height(1080.0);
                    get_popup(popup_settings)
                }
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
