// SPDX-License-Identifier: GPL-3.0-only
//
// Reusable widget helpers and styles for the applet UI.

use crate::calendar::CalendarInfo;
use crate::fl;
use crate::formatting::parse_hex_color;
use cosmic::cosmic_theme;
use cosmic::iced::Length;
use cosmic::iced_core::id;
use cosmic::prelude::*;
use cosmic::widget;

/// Get display format labels for the dropdown (must be called at runtime for localization)
pub fn display_format_options() -> Vec<String> {
    vec![
        fl!("display-format-day-time"),
        fl!("display-format-relative"),
    ]
}

/// Get theme spacing values
pub fn spacing() -> cosmic_theme::Spacing {
    cosmic::theme::spacing()
}

/// Generate a unique ID for an email input field
pub fn email_input_id(idx: usize) -> id::Id {
    id::Id::new(format!("email_input_{}", idx))
}

/// Secondary text style for dimmed/muted text appearance
pub fn secondary_text_style(theme: &cosmic::Theme) -> cosmic::iced_widget::text::Style {
    cosmic::iced_widget::text::Style {
        color: Some(theme.cosmic().palette.neutral_6.into()),
    }
}

/// Featured item button style: transparent background, rounded rect hover matching menu items
pub fn featured_button_style() -> cosmic::theme::Button {
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

/// Create a calendar color indicator dot widget with optional tooltip showing calendar name
pub fn calendar_color_dot<'a, M: 'a>(
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
