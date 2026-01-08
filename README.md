# Next Meeting for COSMIC

A beautiful applet for the [COSMIC desktop environment](https://system76.com/cosmic) that displays your next upcoming calendar event right in your panel. Never miss another meeting again! 

It's similar to [Notion Calendar](https://www.notion.so/product/calendar) and [Fantastical](https://flexibits.com/fantastical) on Mac OS X and [gnome-next-meeting-applet](https://github.com/chmouel/gnome-next-meeting-applet) on GNOME -- but carefully crafted to fit in perfectly on COSMIC!

![Next Meeting Screenshot](resources/screenshots/panel.png)


## Features

- 📅 **See your next meeting at a glance** — Shows the meeting title, time, and location right in your panel
- 🔗 **One-click join** — Detects video call URLs (Google Meet, Zoom, Teams, Webex, or any other app) and shows a "Join" button.
- 🎚️ **Flexible formatting options**:
    * Show the absolute time or relative time until (e.g. "in 2h 30m").
    * See room names and locations for in-person meetings
    * Indicate which calendar with colored dot
- 🔍 **Smart filtering** — Filter by calendar, all-day events, or your acceptance status
- 🌐 **Works with Evolution** — Works with all your Evolution Data Server calendars (GNOME Online Accounts, local calendars, etc.).


## Installation

Download the latest release from the [Releases page](https://github.com/dangrover/next-meeting-for-cosmic/releases).

### Flatpak (Recommended)

Download the `.flatpak` file, then install it:

```bash
flatpak install --user cosmic-next-meeting.flatpak
```

### Debian/Ubuntu/Pop!_OS

Download the `.deb` file, then install it:

```bash
sudo apt install ./cosmic-next-meeting_*.deb
```

After installing, restart your COSMIC panel or log out and back in.

## Requirements

- COSMIC Desktop Environment
- Evolution Data Server (for calendar access)
- Calendars configured via GNOME Online Accounts or Evolution

## Development

### Building

```bash
just build-release    # Build release binary
just run              # Build and run
just dev              # Build, install, and reload panel
just check            # Run clippy lints
```

### Packaging

For distribution packaging, vendor dependencies and use the provided install targets:

```bash
just vendor
just build-vendored
just rootdir=debian/cosmic-next-meeting prefix=/usr install
```

### Translating

Localization uses [Fluent](https://projectfluent.org/). Translation files are in the [i18n](./i18n) directory. To add a new language:

1. Copy the `i18n/en` directory to your [ISO 639-1 language code](https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes)
2. Translate the messages in the `.ftl` file
3. Submit a pull request

## License

GPL-3.0-only
