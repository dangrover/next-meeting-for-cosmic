# Next Meeting for COSMIC

A beautiful panel applet for the [COSMIC desktop environment](https://system76.com/cosmic) that displays your next upcoming calendar event right in your panel (or dock). Never miss another meeting again! 

It's similar to [Notion Calendar](https://www.notion.so/product/calendar) and [Fantastical](https://flexibits.com/fantastical) on Mac OS X and [gnome-next-meeting-applet](https://github.com/chmouel/gnome-next-meeting-applet) on GNOME -- but carefully crafted to fit in perfectly on COSMIC!

![Next Meeting Screenshot](resources/screenshots/panel.png)


## Features

- 📅 **See your next meeting at a glance** — Shows the meeting title, time, and location right in your panel
- 🔗 **One-click join** — Detects video call URLs and shows a "Join" button (Google Meet, Zoom, Teams, Webex out of the box, plus any others you add). 
- 🎚️ **Flexible formatting options**:
    * Show the absolute time or relative time until (e.g. "in 2h 30m").
    * See room names and locations for in-person meetings
    * Indicate which calendar with colored dot (e.g. to distinguish work vs personal). 
- 🔍 **Smart filtering** — Filter by calendar, all-day events, or your acceptance status
- 🌐 **Works with Evolution** — Works with all your Evolution Data Server calendars (GNOME Online Accounts, local calendars, etc.).


## Installation

### Flatpak Repository (Recommended)

Add the Flatpak repository to get automatic updates:

```bash
flatpak remote-add --user --if-not-exists next-meeting https://dangrover.github.io/next-meeting-for-cosmic/index.flatpakrepo
flatpak install --user next-meeting com.dangrover.next-meeting-app
```

### Flatpak Bundle

Alternatively, download the `.flatpak` bundle from the [Releases page](https://github.com/dangrover/next-meeting-for-cosmic/releases):

```bash
flatpak install --user cosmic-next-meeting-x86_64.flatpak
```

### Debian/Ubuntu/Pop!_OS

Download the `.deb` file from the [Releases page](https://github.com/dangrover/next-meeting-for-cosmic/releases):

```bash
sudo apt install ./cosmic-next-meeting_*.deb
```

After installing, you should be able to enable it in Settings > Desktop > Panel > Applets.

### Setting Up Calendars

* COSMIC DE doesn't have any native calendar app or way to set up online calendars (though one is [apparently in progress](https://github.com/cosmic-utils/calendar)). You can do this through Evolution or the Online Accounts setting in GNOME. 
    * If you're using PopOS, Evolution will probably be easier than using Online Accounts to set up calendars. 
* This app is agnostic to what calendar app you use, but it gets its data from EDS (Evolution Data Server). 
    * Other, non-EDS calendars (like Thunderbird) won't work as a data source. But you can set up the same calendars in EDS and still open other calendar apps from the applet. The applet will honor whatever calendar app is configured as the system calendar app. 
    * The applet reads from cached events. If EDS syncs your online calendars, it will see the updates. You can optionally enable a setting to automatically tell EDS to fetch stuff from online calendars.


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
