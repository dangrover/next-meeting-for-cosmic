# AGENTS GUIDE

These instructions apply to all work in this repository. They summarize build/test commands, style, and repo workflows for agentic coders. Base assumptions come from `CLAUDE.md` and project conventions.

## Build, Lint, Test
- Preferred dev command: `just dev` (builds release, installs to `~/.local/bin/meeting`, restarts COSMIC panel).
- Standard builds: `just build-release` (default), `just build-debug` for debug profile.
- Run app quickly: `just run` (release profile with `RUST_BACKTRACE=full`).
- Lint: `just check` (runs `cargo clippy --all-features -W clippy::pedantic`). Use `just check -- --fix` only if fixing lint issues intentionally.
- JSON lint output: `just check-json`.
- Clean: `just clean` or `just clean-dist` (also clears vendored deps).
- Packaging helpers: `just build-vendored`, `just build-flatpak`, `just install-flatpak`, `just build-deb`, `just vendor`, `just update-flatpak-sources`.
- Reload panel manually if needed: `just reload-applet` (restarts `cosmic-panel`).

## Running Tests
- There are currently no Rust tests; add focused tests near touched code when useful.
- Run full suite: `cargo test` (respect `CARGO_TARGET_DIR` if set).
- Run single test or module: `cargo test <pattern>` (e.g., `cargo test meeting_url`); list with `cargo test -- --list`.
- For doctests in a single file: `cargo test --doc --lib -- <pattern>`.
- Use `CARGO_TARGET_DIR=target` unless a different dir is already configured.

## Tooling and Versions
- Rust edition 2024; rely on standard `rustfmt` defaults unless file shows a different style.
- No repo `rustfmt.toml`; use default formatter behavior.
- No Cursor or Copilot rules found (no `.cursor/rules`, `.cursorrules`, or `.github/copilot-instructions.md`).
- `libcosmic` is a git dependency; keep `Cargo.lock` unchanged unless intentionally updating deps.
- Use `tokio` with full features; prefer async patterns already in code (background tasks, subscriptions).

## Branching and Workflow
- Do not work directly on `main`. Switch to `dev` or a feature branch before changes. If on `main`, stash and move.
- Before commits: run `cargo fmt` and `just check` at minimum. For functional changes, use `just dev` so the installed binary reflects updates.
- Keep commits atomic and scoped. Do not push unless user requests.
- Releases are tagged `vX.Y.Z`; tags without `v` do not trigger release workflow.

## Platform Compatibility
- Target: COSMIC desktop across distros. Assume systemd for best experience, but provide graceful fallbacks for non-systemd (e.g., if `org.freedesktop.login1` unavailable, avoid panic and skip wake/unlock hooks).
- D-Bus services used (declared in Flatpak manifest): `org.gnome.evolution.dataserver.Calendar8`, `org.gnome.evolution.dataserver.Sources5`, `org.gnome.OnlineAccounts`, `org.freedesktop.login1`. Always check availability and degrade gracefully.
- Use XDG base dirs; never hardcode `~/.config` paths beyond allowed ones (current access to `~/.config/cosmic` and `~/.config/evolution`).

## Flatpak Notes
- Flatpak sandbox limits D-Bus and filesystem. Any new D-Bus access must be added to `com.dangrover.next-meeting-app.json` using `--talk-name` / `--system-talk-name`.
- Filesystem access is restricted; favor existing allowed dirs. Provide fallbacks if feature cannot run inside sandbox.
- Test both native and Flatpak flows; when a capability is missing in Flatpak, fail soft with user-facing messaging or no-op.

## COSMIC UI Patterns
- Follow COSMIC applet layout: outer column with `.padding([8, 0])`; use `cosmic::applet::menu_button()` for clickable rows, `cosmic::applet::padded_control()` for headings/non-interactive areas, and `cosmic::applet::padded_control(widget::divider::horizontal::default()).padding([space_xxs, space_s])` for dividers.
- Settings lists should use `widget::list_column()` with dividers matching peers.
- Ensure actions like "Join" buttons align with existing spacing and icon usage in `widgets` module.

## Localization
- Localization via Fluent in `i18n/<lang>/meeting.ftl`. Use `fl!("message-id")` macros for strings; avoid hardcoded text.
- When adding strings, update `i18n/en/meeting.ftl` and mirror to other languages as needed. Keep placeholders and attributes consistent.
- Initialize localization using `i18n::init(&requested_languages)` pattern; do not bypass requester.

## Imports and Formatting
- Run `cargo fmt` after changes. Keep `use` statements grouped by crate: std, external, crate modules, then self modules; avoid unused imports.
- Prefer fully qualified module paths in code over glob imports unless existing file uses globs intentionally.
- Order dependencies consistently; prefer alphabetical within groups.
- Keep lines readable; follow rustfmt on max width; avoid manual alignment that rustfmt will undo.

## Types and Naming
- Use descriptive names; avoid single-letter locals except common indices. Favor `meeting`, `config_ctx`, `runtime`, etc.
- Use `const` for shared literals (e.g., app IDs, URL schemes).
- Prefer `&str` over `String` when borrowing suffices; clone only when required.
- For enums/structs, derive common traits (`Debug`, `Clone`, `Serialize`/`Deserialize` if appropriate) consistent with existing patterns.
- Follow Rust naming: `CamelCase` for types/traits, `snake_case` for fns/vars, `SCREAMING_SNAKE_CASE` for constants.

## Error Handling
- Favor early returns with `let-else` for clarity, as seen in `join_next_meeting` and CLAUDE guidance.
- Convert fallible ops into user-friendly results; avoid `unwrap`/`expect` unless truly unreachable. Use `.ok()`, `.map_or_else`, `.and_then`, `.is_some_and` per pedantic guidance.
- When interacting with D-Bus or filesystem, log errors and continue rather than crashing; prefer `Result` propagation with context where useful.
- Use `i32::from(!success)` style for command exit codes as in main.

## Clippy Pedantic Expectations
- Format strings: `format!("{value}")` instead of `format!("{}", value)`.
- Use `.cloned()` instead of `.map(String::clone)`; use `.map_or_else(default_fn, transform_fn)` for fallbacks.
- Boolean checks: use `.is_some_and(predicate)` / `.is_ok_and(predicate)` rather than nesting.
- Merge identical match arms; place wildcard last.
- In doc comments, wrap identifiers in backticks (e.g., `` `CalendarInfo` ``).
- Prefer method references like `String::as_str` when possible.

## Async and Concurrency
- Use `tokio` runtime as in `join_next_meeting`; avoid spawning nested runtimes.
- For periodic tasks, reuse existing subscription/interval patterns in `app.rs` instead of ad-hoc timers.
- When blocking operations are required, consider `spawn_blocking` to avoid stalling async tasks.

## Data Flow Expectations
- Initial fetch happens in `AppModel::init()`; background refresh every 60s; wake/unlock signals trigger immediate refresh. Preserve this behavior when modifying refresh logic.
- Calendar parsing uses EDS iCalendar output; filtering should prefer soonest future event and respect config filters (calendars, acceptance status, all-day, URL patterns).
- UI messages should flow through `update()` message handlers; avoid side-effects in view code.

## Configuration
- Config uses `cosmic_config` derive macros; maintain versioning via `Config::VERSION`. When adding fields, provide defaults/migrations.
- Respect enabled calendars, additional emails, and meeting URL patterns from config when filtering/joining meetings.

## Filesystem and Paths
- Always use XDG dirs (`xdg` crate) instead of hardcoded paths. Current read-only allowances: `~/.config/cosmic`, `~/.config/evolution`.
- Avoid writing outside permitted dirs; for caches, prefer `xdg::BaseDirectories::with_prefix`.

## Dependencies and Patching
- `libcosmic` comes from git. If temporarily patching to local path, use commented `[patch]` block in `Cargo.toml` and remove before committing.
- Keep `Cargo.lock` aligned with `Cargo.toml`; do not vendor unless running packaging recipes.

## Logging and Diagnostics
- Favor concise, user-relevant logging. Avoid noisy debug prints. Use structured errors around D-Bus failures.
- For debug binaries (e.g., `debug_calendars`), keep output stable and informative.

## Security and Secrets
- Do not hardcode credentials or tokens. This app relies on Evolution/GOA for auth; never store secrets in code or repo.
- When handling meeting URLs, avoid logging full URLs to protect privacy.

## Adding Tests
- If adding tests, prefer unit tests near the module (e.g., `mod tests` at bottom). Keep them deterministic; avoid live D-Bus/network calls—mock or fixture iCal data instead.
- For integration-style checks, consider feature-gated tests to avoid requiring COSMIC/EDS at CI time.

## Documentation
- Update README or in-file doc comments when changing build steps or user-visible behavior.
- Keep AGENTS.md current when new tooling or rules appear (Cursor/Copilot/etc.).

## Contribution Checklist (quick)
- Switch off `main`; work on `dev`/feature branch.
- Make change; run `cargo fmt`, `just check`, and where relevant `just dev` to refresh installed binary.
- Add/adjust tests when logic changes; ensure they pass or explain gaps.
- Ensure D-Bus/Flatpak fallbacks remain graceful; no panics on missing services.
- Keep localization keys updated and translated for new strings.

## When in Doubt
- Mirror patterns in `app.rs`, `calendar.rs`, and `widgets` for style and flow.
- Prefer small, composable functions; avoid sprawling `update` arms by delegating to helpers.
- Ask before adding new heavy dependencies or changing Flatpak permissions.

## Testing Discipline
- Prefer specific targets: `cargo test path::to::mod::test_name` for minimal scope.
- If adding new logic, pair with a nearby unit test when practical.
- Avoid masking flakes with retries; document any known instability instead.
- Do not rely on live D-Bus/network in tests; prefer fixtures/mocks.
- Run `cargo fmt` before `just check` to reduce lint noise.

## Debugging and Inspection
- Use `just run -- --join-next` to exercise the CLI join path.
- `cargo run --bin debug_calendars -- --help` lists options for inspecting calendar data.
- Keep temporary debug logging local; remove or gate behind feature flags before merging.

## Git Hygiene
- Stage only relevant files; avoid committing generated assets or caches.
- Keep commit messages short, imperative, and scoped (e.g., "Add meeting filter").
- Never commit secrets or credential-bearing files; skip `.env`-style artifacts.

## Documentation and Comments
- Favor doc comments that explain intent and constraints, not restating names.
- Wrap identifiers in backticks inside doc comments per clippy pedantic guidance.
- Keep user-facing strings in Fluent files; avoid hardcoded UI text.

## Validation and Manual Checks
- After UI changes, sanity-check in a COSMIC session with `just dev` or `just run`.
- For Flatpak paths, confirm permissions and messaging when a capability is missing.
- If changing keyboard shortcut behavior, re-verify `--join-next` exit codes and return values.
