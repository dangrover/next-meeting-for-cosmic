// SPDX-License-Identifier: MPL-2.0

use crate::calendar::{CalendarInfo, Meeting};
use crate::config::Config;
use crate::fl;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::{window::Id, Length, Limits, Subscription};
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::prelude::*;
use cosmic::widget;
use futures_util::SinkExt;

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
    /// The next meeting to display.
    next_meeting: Option<Meeting>,
    /// Available calendars from Evolution Data Server.
    available_calendars: Vec<CalendarInfo>,
    /// Config context for saving changes.
    config_context: Option<cosmic_config::Config>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    UpdateConfig(Config),
    MeetingUpdated(Option<Meeting>),
    CalendarsLoaded(Vec<CalendarInfo>),
    ToggleCalendar(String),
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
            async move { crate::calendar::get_next_meeting(&enabled_uids).await },
            |meeting| Message::MeetingUpdated(meeting).into(),
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
        let text = if let Some(meeting) = &self.next_meeting {
            format!("{}", meeting.title)
        } else {
            "No meetings".to_string()
        };
        
        widget::mouse_area(
            widget::container(widget::text(text))
                .padding([8, 12])
                .width(Length::Fixed(200.0))
        )
        .on_press(Message::TogglePopup)
        .into()
    }

    /// The applet's popup window will be drawn using this view method. If there are
    /// multiple poups, you may match the id parameter to determine which popup to
    /// create a view for.
    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let mut content_list = widget::list_column()
            .padding(5)
            .spacing(0);

        // Add section header
        content_list = content_list.add(
            widget::container(widget::text::body(fl!("calendars-section")))
                .padding([8, 12])
        );

        // Add toggle for each available calendar
        for calendar in &self.available_calendars {
            let is_enabled = self.config.enabled_calendar_uids.is_empty()
                || self.config.enabled_calendar_uids.contains(&calendar.uid);

            let uid = calendar.uid.clone();
            content_list = content_list.add(widget::settings::item(
                &calendar.display_name,
                widget::toggler(is_enabled).on_toggle(move |_| Message::ToggleCalendar(uid.clone())),
            ));
        }

        self.core.applet.popup_container(content_list).into()
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

        Subscription::batch(vec![
            // Periodically refresh calendar data
            Subscription::run_with_id(
                std::any::TypeId::of::<CalendarSubscription>(),
                cosmic::iced::stream::channel(4, move |mut channel| async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                    loop {
                        interval.tick().await;
                        let meeting = crate::calendar::get_next_meeting(&enabled_uids).await;
                        let _ = channel.send(Message::MeetingUpdated(meeting)).await;
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
            Message::MeetingUpdated(meeting) => {
                self.next_meeting = meeting;
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
                return Task::perform(
                    async move { crate::calendar::get_next_meeting(&enabled_uids).await },
                    |meeting| Message::MeetingUpdated(meeting).into(),
                );
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
