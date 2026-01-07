// SPDX-License-Identifier: MPL-2.0

use crate::calendar::{CalendarInfo, Meeting};
use crate::config::{Config, DisplayFormat};
use crate::fl;
use cosmic::cosmic_config::{self, ConfigGet, CosmicConfigEntry};
use cosmic::cosmic_theme;
use cosmic::iced::{window::Id, Length, Limits, Subscription};
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::prelude::*;
use cosmic::widget;
use futures_util::SinkExt;

/// Display format labels for the dropdown
const DISPLAY_FORMAT_OPTIONS: &[&str] = &[
    "Day & time",
    "Relative time",
];

/// Upcoming events count options (0-10)
const UPCOMING_COUNT_OPTIONS: &[&str] = &[
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10",
];

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

/// Get theme spacing values
fn spacing() -> cosmic_theme::Spacing {
    cosmic::theme::spacing()
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
        use chrono::Local;
        let space = spacing();
        let now = Local::now();

        let mut content = widget::column::with_capacity(4)
            .padding(space.space_xs)
            .spacing(space.space_xs);

        if let Some(meeting) = self.upcoming_meetings.first() {
            let duration = meeting.start.signed_duration_since(now);
            let relative = format_relative_time(duration);
            let time_str = format_time(&meeting.start, true);

            // Next meeting section (separate group)
            let next_meeting_block = widget::column::with_capacity(3)
                .push(widget::text::title4(&meeting.title))
                .push(widget::text::body(time_str))
                .push(widget::text::body(relative))
                .spacing(space.space_xxxs)
                .width(Length::Fill)
                .apply(widget::container)
                .padding(space.space_xs)
                .width(Length::Fill)
                .class(cosmic::theme::Container::List);

            content = content.push(next_meeting_block);

            // Upcoming events section (separate group)
            let upcoming_count = self.config.upcoming_events_count as usize;
            if upcoming_count > 0 && self.upcoming_meetings.len() > 1 {
                let mut upcoming_block = widget::column::with_capacity(upcoming_count)
                    .spacing(space.space_xxxs)
                    .width(Length::Fill);

                for meeting in self.upcoming_meetings.iter().skip(1).take(upcoming_count) {
                    let title = if meeting.title.len() > 25 {
                        format!("{}...", &meeting.title[..22])
                    } else {
                        meeting.title.clone()
                    };
                    let time_str = format_time(&meeting.start, false);

                    // Get secondary text color from theme
                    let secondary_color = cosmic::theme::active().cosmic().palette.neutral_6;

                    upcoming_block = upcoming_block.push(
                        widget::row::with_capacity(3)
                            .push(widget::text::caption(title))
                            .push(widget::horizontal_space())
                            .push(
                                widget::text::caption(time_str)
                                    .apply(widget::container)
                                    .class(cosmic::theme::Container::custom(move |_| {
                                        cosmic::iced_widget::container::Style {
                                            text_color: Some(secondary_color.into()),
                                            ..Default::default()
                                        }
                                    }))
                            )
                            .spacing(space.space_xs)
                    );
                }

                content = content.push(
                    upcoming_block
                        .apply(widget::container)
                        .padding(space.space_xs)
                        .width(Length::Fill)
                        .class(cosmic::theme::Container::List)
                );
            }
        } else {
            content = content.push(
                widget::text::body(fl!("no-meetings"))
                    .apply(widget::container)
                    .padding(space.space_xs)
                    .class(cosmic::theme::Container::List)
            );
        }

        // Settings navigation row
        content = content.push(
            widget::button::custom(
                widget::row::with_capacity(3)
                    .push(widget::text::body(fl!("settings")))
                    .push(widget::horizontal_space())
                    .push(widget::icon::from_name("go-next-symbolic").size(16))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
                    .padding(space.space_xs)
            )
            .width(Length::Fill)
            .class(cosmic::theme::Button::MenuItem)
            .on_press(Message::Navigate(PopupPage::Settings))
        );

        content.into()
    }

    /// Settings page with back button
    fn view_settings_page(&self) -> Element<'_, Message> {
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
                        .label(fl!("back"))
                        .spacing(space.space_xxxs)
                        .class(cosmic::theme::Button::Link)
                        .on_press(Message::Navigate(PopupPage::Main))
                )
                .push(widget::text::title4(fl!("settings")))
                .spacing(space.space_xxxs)
        );

        // Display format dropdown
        let format_idx = match self.config.display_format {
            DisplayFormat::DayAndTime => Some(0),
            DisplayFormat::Relative => Some(1),
            _ => Some(0), // Default to DayAndTime for legacy values
        };

        content = content.push(
            widget::row::with_capacity(3)
                .push(widget::text::body(fl!("display-format-section")))
                .push(widget::horizontal_space())
                .push(widget::dropdown(DISPLAY_FORMAT_OPTIONS, format_idx, Message::SelectDisplayFormat)
                    .width(Length::Fixed(180.0)))
                .align_y(cosmic::iced::Alignment::Center)
                .width(Length::Fill)
                .apply(widget::container)
                .padding(space.space_xs)
                .width(Length::Fill)
                .class(cosmic::theme::Container::List)
        );

        // Upcoming events count dropdown
        let count_idx = Some(self.config.upcoming_events_count as usize);
        content = content.push(
            widget::row::with_capacity(3)
                .push(widget::text::body(fl!("upcoming-events-section")))
                .push(widget::horizontal_space())
                .push(widget::dropdown(UPCOMING_COUNT_OPTIONS, count_idx, Message::SetUpcomingEventsCount)
                    .width(Length::Fixed(80.0)))
                .align_y(cosmic::iced::Alignment::Center)
                .width(Length::Fill)
                .apply(widget::container)
                .padding(space.space_xs)
                .width(Length::Fill)
                .class(cosmic::theme::Container::List)
        );

        // Calendars navigation row
        let enabled_count = if self.config.enabled_calendar_uids.is_empty() {
            self.available_calendars.len()
        } else {
            self.config.enabled_calendar_uids.len()
        };
        let calendar_summary = format!("{} enabled", enabled_count);

        content = content.push(
            widget::button::custom(
                widget::row::with_capacity(4)
                    .push(widget::text::body(fl!("calendars-section")))
                    .push(widget::horizontal_space())
                    .push(widget::text::body(calendar_summary))
                    .push(widget::icon::from_name("go-next-symbolic").size(16))
                    .spacing(space.space_xs)
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(Length::Fill)
                    .padding(space.space_xs)
            )
            .width(Length::Fill)
            .class(cosmic::theme::Button::MenuItem)
            .on_press(Message::Navigate(PopupPage::Calendars))
        );

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

        // Calendar toggles
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

            content = content.push(
                row
                    .width(Length::Fill)
                    .apply(widget::container)
                    .padding(space.space_xs)
                    .width(Length::Fill)
                    .class(cosmic::theme::Container::List)
            );
        }

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

/// Format a duration as relative time (e.g., "In 2d 3h" or "In 2h 30m")
/// Shows minutes only when the event is within 24 hours
fn format_relative_time(duration: chrono::Duration) -> String {
    let total_minutes = duration.num_minutes();
    if total_minutes < 0 {
        return "Now".to_string();
    }

    let days = total_minutes / (24 * 60);
    let hours = (total_minutes % (24 * 60)) / 60;
    let minutes = total_minutes % 60;

    if days > 0 {
        // More than a day away - show days and hours, skip minutes
        if hours > 0 {
            format!("In {}d {}h", days, hours)
        } else {
            format!("In {}d", days)
        }
    } else if hours > 0 {
        // Within 24 hours - show hours and minutes
        if minutes > 0 {
            format!("In {}h {}m", hours, minutes)
        } else {
            format!("In {}h", hours)
        }
    } else {
        format!("In {}m", minutes)
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
    SetUpcomingEventsCount(usize),
    Navigate(PopupPage),
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
        let space = spacing();
        let text = if let Some(meeting) = self.upcoming_meetings.first() {
            self.format_meeting_text(meeting)
        } else {
            "No meetings".to_string()
        };

        let button = widget::button::custom(
            widget::row::with_capacity(1)
                .push(widget::text(text))
                .padding([space.space_xxxs, space.space_xs])
        )
        .class(cosmic::theme::Button::AppletIcon)
        .on_press(Message::TogglePopup);

        self.core.applet.autosize_window(
            widget::row::with_capacity(1).push(button)
        ).into()
    }

    /// The applet's popup window will be drawn using this view method. If there are
    /// multiple poups, you may match the id parameter to determine which popup to
    /// create a view for.
    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let content: Element<'_, Self::Message> = match self.current_page {
            PopupPage::Main => self.view_main_page(),
            PopupPage::Settings => self.view_settings_page(),
            PopupPage::Calendars => self.view_calendars_page(),
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
            // Periodically refresh calendar data
            Subscription::run_with_id(
                std::any::TypeId::of::<CalendarSubscription>(),
                cosmic::iced::stream::channel(4, move |mut channel| async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                    loop {
                        interval.tick().await;
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
                self.config.upcoming_events_count = count.min(10) as u8;
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
