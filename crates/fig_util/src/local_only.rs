//! Local-only build: autocomplete without Amazon login, chat, or telemetry.

use std::path::PathBuf;

use crate::consts::{
    APP_BUNDLE_ID,
    APP_BUNDLE_NAME,
    APP_PROCESS_NAME,
};

pub const FIGLOCAL_APP_BUNDLE_ID: &str = "dev.figlocal.autocomplete";
pub const FIGLOCAL_APP_BUNDLE_NAME: &str = "figlocal.app";
pub const FIGLOCAL_PRODUCT_NAME: &str = "figlocal";
pub const FIGLOCAL_DESKTOP_PROCESS_NAME: &str = "fig_desktop";

/// User-visible product name in local-only mode.
pub fn product_name() -> &'static str {
    FIGLOCAL_PRODUCT_NAME
}

/// True when built with `local-only` or `FIG_LOCAL_ONLY=1`.
pub fn enabled() -> bool {
    cfg!(feature = "local-only") || std::env::var("FIG_LOCAL_ONLY").as_deref() == Ok("1")
}

/// macOS bundle id for the desktop app in this mode.
pub fn app_bundle_id() -> &'static str {
    if enabled() {
        FIGLOCAL_APP_BUNDLE_ID
    } else {
        APP_BUNDLE_ID
    }
}

/// Installed `.app` directory name under Applications.
pub fn app_bundle_name() -> &'static str {
    if enabled() {
        FIGLOCAL_APP_BUNDLE_NAME
    } else {
        APP_BUNDLE_NAME
    }
}

/// Desktop helper process name (executable basename).
pub fn desktop_process_name() -> &'static str {
    if enabled() {
        FIGLOCAL_DESKTOP_PROCESS_NAME
    } else {
        APP_PROCESS_NAME
    }
}

/// Default install location when not running from inside the bundle.
pub fn default_app_bundle_path() -> Option<PathBuf> {
    if !enabled() {
        return None;
    }
    crate::directories::home_dir()
        .ok()
        .map(|home| home.join("Applications").join(FIGLOCAL_APP_BUNDLE_NAME))
}

/// Local Fig builds are ad-hoc signed; macOS often never marks AX trusted. Allow autocomplete anyway.
pub fn allow_autocomplete_without_accessibility() -> bool {
    enabled()
}
