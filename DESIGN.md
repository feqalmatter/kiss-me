# Design Philosophy

`kiss-me` should stay small, boring, and legible: keep the CLI scriptable before
adding interactive polish, treat the filesystem as the source of truth, and only
add helpers when they remove real repetition or make dangerous operations safer.

The Rust rewrite keeps the core operations independent from the TUI. CLI
commands and UI buttons call the same functions, so the interface is not the
architecture. The TUI is a convenience layer: a mod list, command buttons, and an
output pane.

Configuration is profile-aware but intentionally simple. A profile is just the
set of paths and Nexus game domain needed to operate on one game install. The UI
uses one active profile at a time rather than trying to become a full multi-game
manager.

Prefer clear command names, predictable exit behavior, and plain status messages.
Avoid compatibility shims for unreleased behavior, avoid new dependencies unless
they unlock obvious value, and keep filesystem operations explicit where external
tools such as `cp -a --reflink=auto` preserve game trees better than bespoke copy
code.
