// This is needed for objc
#![allow(unexpected_cfgs)]
#![allow(deprecated)]

use std::ffi::CString;
use std::path::Path;
use std::slice;
use std::sync::atomic::{
    AtomicBool,
    AtomicU64,
    Ordering,
};
use std::sync::{
    Arc,
    LazyLock,
    Mutex,
    RwLock,
};
use std::time::Duration;

use accessibility_sys::{
    AXError,
    AXIsProcessTrusted,
    AXUIElementCreateSystemWide,
    AXUIElementSetMessagingTimeout,
    pid_t,
};
use anyhow::Context;
use cocoa::base::{
    NO,
    YES,
    id,
};
use core_graphics::display::CGRect;
use core_graphics::window::CGWindowID;
use fig_integrations::input_method::InputMethod;
use fig_proto::fig::{
    AccessibilityChangeNotification,
    Notification,
    NotificationType,
};
use fig_proto::local::caret_position_hook::Origin;
use fig_util::Terminal;
use macos_utils::accessibility::accessibility_is_enabled;
use macos_utils::caret_position::{
    CaretPosition,
    get_caret_position,
};
use macos_utils::applications::frontmost_application;
use macos_utils::window_server::{
    CGWindowInfo,
    CGWindowLevelForKey,
    UIElement,
    largest_window_for_pid,
    window_bounds_by_id,
};
use macos_utils::{
    NotificationCenter,
    WindowServer,
    WindowServerEvent,
};
use objc::declare::MethodImplementation;
use objc::runtime::{
    BOOL,
    Class,
    Object,
    Sel,
    class_addMethod,
    objc_getClass,
};
use objc::{
    Encode,
    EncodeArguments,
    Encoding,
    msg_send,
    sel,
    sel_impl,
};
use objc2_foundation::{
    NSDictionary,
    NSOperationQueue,
    ns_string,
};
use serde::Serialize;
use tao::dpi::{
    LogicalPosition,
    LogicalSize,
    Position,
    Size,
};
use tao::platform::macos::{
    ActivationPolicy,
    EventLoopWindowTargetExtMacOS,
    WindowExtMacOS as _,
};
use tracing::{
    debug,
    error,
    trace,
    warn,
};

use super::{
    PlatformBoundEvent,
    PlatformWindow,
};
use crate::event::{
    Event,
    WindowEvent,
    WindowPosition,
};
use crate::protocol::icons::{
    AssetKind,
    AssetSpecifier,
    ProcessedAsset,
};
use crate::utils::Rect;
use crate::webview::notification::WebviewNotificationsState;
use crate::webview::{
    FIGTERM_STATE,
    FigIdMap,
    GLOBAL_PROXY,
    WindowId,
};
use crate::{
    AUTOCOMPLETE_ID,
    autocomplete_window_title,
    DASHBOARD_ID,
    EventLoopProxy,
    EventLoopWindowTarget,
};

pub const DEFAULT_CARET_WIDTH: f64 = 10.0;

// See for other window level keys
// https://github.com/phracker/MacOSX-SDKs/blob/master/MacOSX10.8.sdk/System/Library/Frameworks/CoreGraphics.framework/Versions/A/Headers/CGWindowLevel.h
#[allow(non_upper_case_globals)]
const kCGFloatingWindowLevelKey: i32 = 5;

static UNMANAGED: Unmanaged = Unmanaged {
    event_sender: RwLock::new(Option::<EventLoopProxy>::None),
    window_server: RwLock::new(Option::<Arc<Mutex<WindowServer>>>::None),
};

static ACCESSIBILITY_ENABLED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(accessibility_is_enabled()));

static MACOS_VERSION: LazyLock<semver::Version> = LazyLock::new(|| {
    let version = macos_utils::os::OperatingSystemVersion::get();
    semver::Version::new(version.major() as u64, version.minor() as u64, version.patch() as u64)
});

pub static ACTIVATION_POLICY: Mutex<ActivationPolicy> = Mutex::new(ActivationPolicy::Regular);

#[allow(dead_code)]
pub fn is_ventura() -> bool {
    MACOS_VERSION.major >= 13
}

struct Unmanaged {
    event_sender: RwLock<Option<EventLoopProxy>>,
    window_server: RwLock<Option<Arc<Mutex<WindowServer>>>>,
}

/// Tracks terminal window + caret so autocomplete can follow window moves.
#[derive(Debug, Clone, Copy)]
struct TerminalFollowState {
    window_id: Option<CGWindowID>,
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    caret_x: f64,
    caret_y: f64,
    caret_w: f64,
    caret_h: f64,
}

#[derive(Debug, Serialize)]
pub struct PlatformStateImpl {
    #[serde(skip)]
    proxy: EventLoopProxy,
    #[serde(skip)]
    focused_window: Mutex<Option<PlatformWindowImpl>>,
    /// Last terminal bounds/caret used to follow the window while autocomplete is open.
    #[serde(skip)]
    follow_state: Mutex<Option<TerminalFollowState>>,
    /// Bumped to cancel the in-flight follow poller.
    #[serde(skip)]
    follow_generation: AtomicU64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PlatformWindowImpl {
    window_id: CGWindowID,
    ui_element: UIElement,
    x_term_tree_cache: Option<Vec<UIElement>>,
    pub bundle_id: String,
    pub pid: pid_t,
}

impl From<CGRect> for Rect {
    fn from(cgr: CGRect) -> Rect {
        Rect {
            position: LogicalPosition::new(cgr.origin.x, cgr.origin.y).into(),
            size: LogicalSize::new(cgr.size.width, cgr.size.height).into(),
        }
    }
}

impl PlatformWindowImpl {
    pub fn new(bundle_id: String, pid: pid_t, ui_element: UIElement) -> Result<Self, AXError> {
        let window_id = unsafe { ui_element.get_window_id()? };
        Ok(Self {
            window_id,
            ui_element,
            pid,
            x_term_tree_cache: None,
            bundle_id,
        })
    }

    pub fn get_window_id(&self) -> CGWindowID {
        self.window_id
    }

    pub fn bundle_id(&self) -> &str {
        self.bundle_id.as_str()
    }

    pub fn get_bounds(&self) -> Option<CGRect> {
        let info = self.ui_element.window_info(false)?;
        Some(info.bounds)
    }

    pub fn get_level(&self) -> Option<i64> {
        // We grab all the windows since we don't want this to fail and are more fine if it's slow
        let info = self.ui_element.window_info(true)?;
        Some(info.level)
    }

    pub fn get_x_term_cursor_elem(&mut self) -> Option<UIElement> {
        let tree = self
            .x_term_tree_cache
            .as_ref()
            .and_then(|cached| {
                debug!("About to walk through {:?}", cached.len());
                let result: Option<Vec<UIElement>> =
                    cached.iter().fold(None::<Vec<UIElement>>, |accum, item| match accum {
                        Some(mut x) => {
                            x.push(item.clone());
                            Some(x)
                        },
                        None => item.find_x_term_caret_tree().ok(),
                    });
                result
            })
            .or_else(|| self.ui_element.find_x_term_caret_tree().ok());

        self.x_term_tree_cache = tree;
        self.x_term_tree_cache.as_ref()?.first().cloned()
    }
}

impl PlatformStateImpl {
    pub(super) fn new(proxy: EventLoopProxy) -> Self {
        let focused_window: Option<PlatformWindowImpl> = None;
        Self {
            proxy,
            focused_window: Mutex::new(focused_window),
            follow_state: Mutex::new(None),
            follow_generation: AtomicU64::new(0),
        }
    }

    //
    fn count_args(sel: Sel) -> usize {
        sel.name().chars().filter(|&c| c == ':').count()
    }

    fn method_type_encoding(ret: &Encoding, args: &[Encoding]) -> CString {
        let mut types = ret.as_str().to_owned();
        // First two arguments are always self and the selector
        types.push_str(<*mut Object>::encode().as_str());
        types.push_str(Sel::encode().as_str());
        types.extend(args.iter().map(|e| e.as_str()));
        CString::new(types).unwrap()
    }

    // Add an implementation for an ObjC selector, which will override default WKWebView & WryWebView
    // behavior
    fn override_webview_method<F>(sel: Sel, func: F)
    where
        F: MethodImplementation<Callee = Object>,
    {
        // https://github.com/tauri-apps/wry/blob/17d324b70e4d580c43c9d4ab37bd265005356bf4/src/webview/wkwebview/mod.rs#L258
        Self::override_objc_class_method("WryWebView", sel, func);
    }

    fn override_app_delegate_method<F>(sel: Sel, func: F)
    where
        F: MethodImplementation<Callee = Object>,
    {
        // https://github.com/tauri-apps/tao/blob/75eb0c1e7e83a766af0e083ce09c761d1974cde4/src/platform_impl/macos/app_delegate.rs#L42
        Self::override_objc_class_method("TaoAppDelegateParent", sel, func);
    }

    fn override_objc_class_method<F>(class: &str, sel: Sel, func: F)
    where
        F: MethodImplementation<Callee = Object>,
    {
        let encs = F::Args::encodings();
        let encs = encs.as_ref();
        let sel_args = Self::count_args(sel);
        assert!(
            sel_args == encs.len(),
            "Selector accepts {} arguments, but function accepts {}",
            sel_args,
            encs.len(),
        );

        let types = Self::method_type_encoding(&F::Ret::encode(), encs);

        let name = CString::new(class).unwrap();

        let res = unsafe {
            let cls = objc_getClass(name.as_ptr()) as *mut Class;
            class_addMethod(cls, sel, func.imp(), types.as_ptr())
        };
        trace!(%class, sel =% sel.name(), %res, "class_addMethod");
    }

    pub(super) fn handle(
        self: &Arc<Self>,
        event: PlatformBoundEvent,
        window_target: &EventLoopWindowTarget,
        window_map: &FigIdMap,
        notifications_state: &Arc<WebviewNotificationsState>,
    ) -> anyhow::Result<()> {
        debug!("Handling platform event: {:?}", event);
        match event {
            PlatformBoundEvent::Initialize => {
                unsafe {
                    if AXIsProcessTrusted() {
                        // This prevents Fig from becoming unresponsive if one of the applications
                        // we are tracking becomes unresponsive.
                        AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), 0.25);
                    }
                }

                UNMANAGED.event_sender.write().unwrap().replace(self.proxy.clone());
                let (tx, rx) = flume::unbounded::<WindowServerEvent>();

                UNMANAGED
                    .window_server
                    .write()
                    .unwrap()
                    .replace(Arc::new(Mutex::new(WindowServer::new(tx))));

                let accessibility_proxy = self.proxy.clone();
                let mut distributed = NotificationCenter::distributed_center();
                let ax_notification_name = ns_string!("com.apple.accessibility.api");
                let queue = unsafe { NSOperationQueue::new() };
                distributed.subscribe(ax_notification_name, Some(&queue), move |_| {
                    accessibility_proxy
                        .clone()
                        .send_event(Event::PlatformBoundEvent(
                            PlatformBoundEvent::AccessibilityUpdateRequested,
                        ))
                        .ok();
                });

                let observer_proxy = self.proxy.clone();
                tokio::runtime::Handle::current().spawn(async move {
                    while let std::result::Result::Ok(result) = rx.recv_async().await {
                        let mut events: Vec<Event> = vec![];

                        match result {
                            WindowServerEvent::FocusChanged { window, app } => {
                                // Delayed focus events (e.g. Cursor re-activation) were hiding the popup right after Show.
                                let hide_on_focus = !fig_util::local_only::enabled();
                                if hide_on_focus {
                                    events.push(Event::WindowEvent {
                                        window_id: AUTOCOMPLETE_ID,
                                        window_event: WindowEvent::Hide,
                                    });
                                }

                                if let Ok(window) = PlatformWindowImpl::new(app.bundle_id, app.pid, window) {
                                    events.push(Event::PlatformBoundEvent(
                                        PlatformBoundEvent::ExternalWindowFocusChanged { window },
                                    ));
                                }
                            },
                            WindowServerEvent::WindowDestroyed { app } => {
                                events.push(Event::PlatformBoundEvent(PlatformBoundEvent::WindowDestroyed { app }));
                            },
                            WindowServerEvent::ActiveSpaceChanged { is_fullscreen } => {
                                events.extend([
                                    Event::WindowEvent {
                                        window_id: AUTOCOMPLETE_ID.clone(),
                                        window_event: WindowEvent::Hide,
                                    },
                                    Event::PlatformBoundEvent(PlatformBoundEvent::FullscreenStateUpdated {
                                        fullscreen: is_fullscreen,
                                        dashboard_visible: None,
                                    }),
                                ]);
                            },
                            WindowServerEvent::RequestCaretPositionUpdate => {
                                events.push(Event::PlatformBoundEvent(
                                    PlatformBoundEvent::CaretPositionUpdateRequested,
                                ));
                            },
                        };

                        for event in events {
                            if let Err(e) = observer_proxy.send_event(event) {
                                warn!("Error sending event: {e:?}");
                            }
                        }
                    }
                });

                Ok(())
            },
            PlatformBoundEvent::InitializePostRun => {
                fn to_s<'a>(nsstring_obj: *mut Object) -> Option<&'a str> {
                    const UTF8_ENCODING: libc::c_uint = 4;

                    let bytes = unsafe {
                        let length = msg_send![nsstring_obj, lengthOfBytesUsingEncoding: UTF8_ENCODING];
                        let utf8_str: *const u8 = msg_send![nsstring_obj, UTF8String];
                        slice::from_raw_parts(utf8_str, length)
                    };
                    std::str::from_utf8(bytes).ok()
                }

                extern "C" fn should_delay_window_ordering(this: &Object, _cmd: Sel, _event: id) -> BOOL {
                    debug!("should_delay_window_ordering");

                    unsafe {
                        let window: id = msg_send![this, window];
                        let title: id = msg_send![window, title];

                        // TODO: implement better method for determining if WebView belongs to autocomplete
                        if let Some(title) = to_s(title) {
                            if title == autocomplete_window_title() {
                                return YES;
                            }
                        }
                    }

                    NO
                }

                extern "C" fn mouse_down(this: &Object, _cmd: Sel, event: id) {
                    let application = Class::get("NSApplication").unwrap();

                    unsafe {
                        let window: id = msg_send![this, window];
                        let title: id = msg_send![window, title];

                        // TODO: implement better method for determining if WebView belongs to autocomplete
                        if let Some(title) = to_s(title) {
                            if title == autocomplete_window_title() {
                                // Prevent clicked window from taking focus
                                let app: id = msg_send![application, sharedApplication];
                                let _: () = msg_send![app, preventWindowOrdering];
                            }
                        }

                        // Invoke superclass implementation
                        let supercls = msg_send![this, superclass];
                        let _: () = msg_send![super(this, supercls), mouseDown: event];
                    }
                }

                // Use objc runtime to override WryWebview methods
                Self::override_webview_method(
                    sel!(shouldDelayWindowOrderingForEvent:),
                    should_delay_window_ordering as extern "C" fn(&Object, Sel, id) -> BOOL,
                );
                Self::override_webview_method(sel!(mouseDown:), mouse_down as extern "C" fn(&Object, Sel, id));

                extern "C" fn application_should_handle_reopen(
                    _this: &Object,
                    _cmd: Sel,
                    _sender: id,
                    _visible_windows: BOOL,
                ) -> BOOL {
                    trace!("application_should_handle_reopen");

                    let proxy = GLOBAL_PROXY.get().unwrap();

                    if let Err(err) = proxy.send_event(Event::WindowEvent {
                        window_id: DASHBOARD_ID,
                        window_event: WindowEvent::Show,
                    }) {
                        warn!(%err, "Error sending event");
                    }

                    YES
                }

                Self::override_app_delegate_method(
                    sel!(applicationShouldHandleReopen:hasVisibleWindows:),
                    application_should_handle_reopen as extern "C" fn(&Object, Sel, id, BOOL) -> BOOL,
                );

                Ok(())
            },
            PlatformBoundEvent::EditBufferChanged => {
                if let Err(err) = self.refresh_window_position() {
                    error!(%err, "Failed to refresh window position");
                }
                Ok(())
            },
            PlatformBoundEvent::ExternalWindowFocusChanged { window } => {
                let current_terminal = Terminal::from_bundle_id(window.bundle_id.clone());
                let level = window.get_level();

                if level == Some(0) {
                    // Checking if IME is installed is async :(
                    let enabled_proxy = self.proxy.clone();
                    tokio::spawn(async move {
                        let is_terminal_disabled = current_terminal.as_ref().is_some_and(|terminal| {
                            fig_settings::settings::get_bool_or(
                                format!("integrations.{}.disabled", terminal.internal_id()),
                                false,
                            )
                        });

                        let terminal_cursor_backing_installed = match current_terminal {
                            Some(terminal) => {
                                if terminal.supports_macos_input_method() {
                                    let input_method: InputMethod = Default::default();
                                    input_method.is_enabled().unwrap_or(false)
                                        && input_method.enabled_for_terminal_instance(&terminal, window.pid)
                                } else {
                                    true
                                }
                            },
                            None => fig_util::local_only::allow_autocomplete_without_accessibility(),
                        };

                        let ax_ok = accessibility_is_enabled()
                            || fig_util::local_only::allow_autocomplete_without_accessibility();
                        let is_enabled = !is_terminal_disabled
                            && terminal_cursor_backing_installed
                            && !fig_settings::settings::get_bool_or("autocomplete.disable", false)
                            && ax_ok;
                        // && fig_request::fig_auth::is_logged_in();

                        enabled_proxy
                            .send_event(Event::WindowEvent {
                                window_id: AUTOCOMPLETE_ID,
                                window_event: WindowEvent::SetEnabled(is_enabled),
                            })
                            .unwrap();
                    });

                    let mut focused = self.focused_window.lock().unwrap();
                    focused.replace(window);
                }

                if let Some(window) = window_map.get(&AUTOCOMPLETE_ID) {
                    let ns_window = window.window.ns_window().cast::<Object>();
                    // Handle iTerm Quake mode by explicitly setting window level. See
                    // https://github.com/gnachman/iTerm2/blob/1a5a09f02c62afcc70a647603245e98862e51911/sources/iTermProfileHotKey.m#L276-L310
                    // for more on window levels.
                    let above = match level {
                        None | Some(0) => unsafe { CGWindowLevelForKey(kCGFloatingWindowLevelKey) as i64 },
                        Some(level) => level,
                    };
                    debug!("Setting window level to {level:?}");
                    let _: () = unsafe { msg_send![ns_window, setLevel: above] };
                }

                Ok(())
            },
            PlatformBoundEvent::CaretPositionUpdateRequested => {
                if let Err(e) = self.refresh_window_position() {
                    debug!(%e, "Failed to refresh window position");
                }
                Ok(())
            },
            PlatformBoundEvent::FullscreenStateUpdated {
                fullscreen,
                dashboard_visible,
            } => {
                let policy = if fullscreen {
                    ActivationPolicy::Accessory
                } else {
                    let dashboard_visible = dashboard_visible.unwrap_or_else(|| {
                        window_map
                            .get(&DASHBOARD_ID)
                            .is_some_and(|window| window.window.is_visible())
                    });

                    if dashboard_visible {
                        ActivationPolicy::Regular
                    } else {
                        ActivationPolicy::Accessory
                    }
                };

                let mut policy_lock = ACTIVATION_POLICY.lock().unwrap();
                if *policy_lock != policy {
                    debug!(?policy, "Setting application policy");
                    *policy_lock = policy;
                    window_target.set_activation_policy_at_runtime(policy);
                }
                Ok(())
            },
            PlatformBoundEvent::AccessibilityUpdateRequested => {
                let proxy = self.proxy.clone();
                tokio::runtime::Handle::current().spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let enabled = accessibility_is_enabled();
                    proxy
                        .send_event(Event::PlatformBoundEvent(PlatformBoundEvent::AccessibilityUpdated {
                            enabled,
                        }))
                        .ok();
                    if enabled {
                        unsafe {
                            // This prevents Fig from becoming unresponsive if one of the applications
                            // we are tracking becomes unresponsive.
                            AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), 0.25);
                        }
                    }
                });

                Ok(())
            },
            PlatformBoundEvent::AccessibilityUpdated { enabled } => {
                let _was_enabled = ACCESSIBILITY_ENABLED.swap(enabled, Ordering::SeqCst);
                // if enabled && !was_enabled {
                //     tokio::runtime::Handle::current().spawn(async move {
                //         fig_telemetry::emit_track(fig_telemetry::TrackEvent::new(
                //             fig_telemetry::TrackEventType::GrantedAXPermission,
                //             fig_telemetry::TrackSource::Desktop,
                //             env!("CARGO_PKG_VERSION").into(),
                //             std::iter::empty::<(&str, &str)>(),
                //         ))
                //         .await
                //         .ok();
                //     });
                // }

                let proxy = self.proxy.clone();
                let notifications_state = notifications_state.clone();
                tokio::spawn(async move {
                    if let Err(err) = notifications_state
                        .broadcast_notification_all(
                            &NotificationType::NotifyOnAccessibilityChange,
                            Notification {
                                r#type: Some(fig_proto::fig::notification::Type::AccessibilityChangeNotification(
                                    AccessibilityChangeNotification { enabled },
                                )),
                            },
                            &proxy,
                        )
                        .await
                    {
                        error!(%err, "Failed to broadcast notification");
                    }
                });

                self.proxy.send_event(Event::ReloadAccessibility).ok();

                Ok(())
            },
            PlatformBoundEvent::AppWindowFocusChanged {
                window_id,
                focused,
                fullscreen,
                visible,
            } => {
                // Update activation policy
                if window_id == DASHBOARD_ID && focused {
                    debug!("Sending FullscreenStateUpdated");
                    self.proxy
                        .send_event(Event::PlatformBoundEvent(PlatformBoundEvent::FullscreenStateUpdated {
                            fullscreen,
                            dashboard_visible: Some(visible),
                        }))
                        .ok();
                }
                Ok(())
            },
            PlatformBoundEvent::WindowDestroyed { app } => {
                let mut focused = self.focused_window.lock().unwrap();
                if let Some(focused_window) = focused.as_ref() {
                    if focused_window.bundle_id() == app.bundle_id {
                        focused.take();
                        self.proxy
                            .send_event(Event::WindowEvent {
                                window_id: AUTOCOMPLETE_ID,
                                window_event: WindowEvent::Hide,
                            })
                            .ok();
                    }
                }
                Ok(())
            },
        }
    }

    pub(super) fn refresh_window_position(&self) -> anyhow::Result<()> {
        let (edit_cursor, shell_pid) = Self::figterm_cursor_and_shell_pid();
        let mut guard = self.focused_window.lock().unwrap();
        let mut active_window = guard.as_mut();

        let bundle_id = active_window
            .as_ref()
            .map(|w| w.bundle_id().to_string())
            .or_else(|| {
                frontmost_application()
                    .and_then(|app| app.bundle_identifier)
                    .filter(|id| id != fig_util::local_only::app_bundle_id() && id != "dev.figlocal.autocomplete")
            });

        let current_terminal = bundle_id.as_deref().and_then(Terminal::from_bundle_id);
        let supports_ime = current_terminal
            .clone()
            .is_some_and(|t| t.supports_macos_input_method());
        let is_xterm = current_terminal.is_some_and(|t| t.is_xterm());

        // figlocal ships without the CodeWhisperer input method — IME path never returns caret coords.
        let use_input_method_caret = !is_xterm && supports_ime && !fig_util::local_only::enabled();
        if use_input_method_caret {
            tracing::debug!("Sending notif com.amazon.codewhisperer.edit_buffer_updated");
            NotificationCenter::distributed_center().post_notification(
                ns_string!("com.amazon.codewhisperer.edit_buffer_updated"),
                &NSDictionary::new(),
            );
            return Ok(());
        }

        let mut caret = if is_xterm {
            active_window
                .as_mut()
                .and_then(|w| w.get_x_term_cursor_elem())
                .and_then(|c| c.frame().ok())
                .map(Rect::from)
        } else {
            self.get_cursor_position()
        };

        let terminal_window = Self::resolve_terminal_window(active_window.as_deref(), shell_pid);
        let bounds = terminal_window.as_ref().map(|w| w.bounds);

        // Without Accessibility, AX caret APIs fail — place on the terminal prompt line via window bounds.
        if caret.is_none() {
            if let Some(bounds) = bounds {
                caret = Some(Self::estimate_caret_on_prompt_line(&bounds, edit_cursor));
            }
        }

        // AX caret frames can stay stale while the terminal window moves; keep relative place.
        if let Some(bounds) = bounds {
            let prev = *self.follow_state.lock().unwrap();
            if let Some(prev) = prev {
                let dx = bounds.origin.x - prev.origin_x;
                let dy = bounds.origin.y - prev.origin_y;
                if dx.abs() > 0.5 || dy.abs() > 0.5 {
                    let expected = LogicalPosition::new(prev.caret_x + dx, prev.caret_y + dy);
                    match caret.as_mut() {
                        Some(c) => {
                            let pos = c.position.to_logical::<f64>(1.0);
                            if (pos.x - prev.caret_x).abs() < 2.0 && (pos.y - prev.caret_y).abs() < 2.0 {
                                c.position = expected.into();
                            }
                        },
                        None => {
                            caret = Some(Rect {
                                position: expected.into(),
                                size: LogicalSize::new(prev.caret_w, prev.caret_h).into(),
                            });
                        },
                    }
                }
            }
        }

        let caret = caret.context("Failed to get cursor position")?;
        debug!(?caret, ?bounds, edit_cursor, "Sending caret update");

        if let Some(info) = terminal_window.as_ref() {
            self.store_follow_state(Some(info.window_id), &info.bounds, &caret);
        } else if let Some(bounds) = bounds {
            self.store_follow_state(None, &bounds, &caret);
        }

        Self::send_caret_geometry(caret.position, caret.size);
        Ok(())
    }

    /// Edit-buffer cursor + shell pid from the most recent figterm session.
    fn figterm_cursor_and_shell_pid() -> (i64, Option<i32>) {
        let Some(state) = FIGTERM_STATE.get() else {
            return (0, None);
        };
        state
            .most_recent()
            .map(|session| {
                let cursor = session.edit_buffer.cursor;
                let pid = session.context.as_ref().and_then(|ctx| ctx.pid);
                (cursor, pid)
            })
            .unwrap_or((0, None))
    }

    /// Resolve the terminal/host window without requiring Accessibility.
    fn resolve_terminal_window(
        active_window: Option<&PlatformWindowImpl>,
        shell_pid: Option<i32>,
    ) -> Option<CGWindowInfo> {
        if let Some(window) = active_window {
            if let Some(bounds) = window.get_bounds() {
                return Some(CGWindowInfo {
                    window_id: window.get_window_id(),
                    bounds,
                    owner_pid: window.pid as u64,
                    level: window.get_level().unwrap_or(0),
                });
            }
            if let Some(info) = largest_window_for_pid(window.pid) {
                return Some(info);
            }
        }

        // Prefer the app that owns the shell (walk parents briefly), else frontmost terminal-like app.
        if let Some(pid) = shell_pid {
            for candidate in Self::pid_and_parents(pid).into_iter().take(8) {
                if let Some(info) = largest_window_for_pid(candidate) {
                    return Some(info);
                }
            }
        }

        let front = frontmost_application()?;
        let bundle = front.bundle_identifier.as_deref().unwrap_or("");
        if bundle == fig_util::local_only::app_bundle_id() || bundle == "dev.figlocal.autocomplete" {
            return None;
        }
        largest_window_for_pid(front.process_identifier)
    }

    fn pid_and_parents(start: i32) -> Vec<i32> {
        let mut out = vec![start];
        let mut current = start;
        for _ in 0..8 {
            let ppid = std::process::Command::new("ps")
                .args(["-o", "ppid=", "-p", &current.to_string()])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<i32>().ok());
            match ppid {
                Some(p) if p > 1 && !out.contains(&p) => {
                    out.push(p);
                    current = p;
                },
                _ => break,
            }
        }
        out
    }

    /// Place caret on the visible prompt line (bottom content area) using buffer cursor for X.
    fn estimate_caret_on_prompt_line(bounds: &CGRect, edit_cursor: i64) -> Rect {
        const LEFT_PAD: f64 = 14.0;
        const BOTTOM_PAD: f64 = 10.0;
        const LINE_H: f64 = 18.0;
        const CHAR_W: f64 = 8.0;

        let col = edit_cursor.max(0) as f64;
        let mut x = bounds.origin.x + LEFT_PAD + col * CHAR_W;
        let max_x = bounds.origin.x + bounds.size.width - 48.0;
        if x > max_x {
            x = max_x;
        }
        if x < bounds.origin.x + LEFT_PAD {
            x = bounds.origin.x + LEFT_PAD;
        }
        let y = bounds.origin.y + bounds.size.height - BOTTOM_PAD - LINE_H;

        Rect {
            position: LogicalPosition::new(x, y).into(),
            size: LogicalSize::new(DEFAULT_CARET_WIDTH, LINE_H).into(),
        }
    }

    /// Remember terminal origin + caret so moves can be applied as a delta.
    fn store_follow_state(&self, window_id: Option<CGWindowID>, bounds: &CGRect, caret: &Rect) {
        let caret_pos = caret.position.to_logical::<f64>(1.0);
        let caret_size = caret.size.to_logical::<f64>(1.0);
        *self.follow_state.lock().unwrap() = Some(TerminalFollowState {
            window_id,
            origin_x: bounds.origin.x,
            origin_y: bounds.origin.y,
            width: bounds.size.width,
            height: bounds.size.height,
            caret_x: caret_pos.x,
            caret_y: caret_pos.y,
            caret_w: caret_size.width,
            caret_h: caret_size.height,
        });
    }

    fn send_caret_geometry(caret_position: Position, caret_size: Size) {
        if let Some(sender) = UNMANAGED.event_sender.read().unwrap().clone() {
            sender
                .send_event(Event::WindowEvent {
                    window_id: AUTOCOMPLETE_ID,
                    window_event: WindowEvent::UpdateWindowGeometry {
                        position: Some(WindowPosition::RelativeToCaret {
                            caret_position,
                            caret_size,
                            origin: Origin::TopLeft,
                        }),
                        size: None,
                        anchor: None,
                        tx: None,
                        dry_run: false,
                    },
                })
                .ok();
        }
    }

    /// Start polling terminal bounds so autocomplete follows window moves/resizes.
    pub(super) fn start_follow_terminal(self: &Arc<Self>) {
        let generation = self.follow_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(33));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if this.follow_generation.load(Ordering::SeqCst) != generation {
                    break;
                }
                if let Err(err) = this.follow_terminal_tick() {
                    trace!(%err, "follow terminal tick skipped");
                }
            }
        });
    }

    /// Stop following the terminal (e.g. when autocomplete hides).
    pub(super) fn stop_follow_terminal(&self) {
        self.follow_generation.fetch_add(1, Ordering::SeqCst);
        *self.follow_state.lock().unwrap() = None;
    }

    /// If terminal window moved, shift caret by the same delta; on resize, full refresh.
    fn follow_terminal_tick(&self) -> anyhow::Result<()> {
        let mut follow = self.follow_state.lock().unwrap();
        let Some(state) = follow.as_mut() else {
            return Ok(());
        };

        let bounds = if let Some(window_id) = state.window_id {
            window_bounds_by_id(window_id).context("Failed to get window bounds by id")?
        } else {
            let guard = self.focused_window.lock().unwrap();
            let active_window = guard.as_ref().context("No active window")?;
            active_window.get_bounds().context("Failed to get window bounds")?
        };

        let dx = bounds.origin.x - state.origin_x;
        let dy = bounds.origin.y - state.origin_y;
        let size_changed =
            (bounds.size.width - state.width).abs() > 0.5 || (bounds.size.height - state.height).abs() > 0.5;

        if size_changed {
            drop(follow);
            return self.refresh_window_position();
        }

        if dx.abs() < 0.5 && dy.abs() < 0.5 {
            return Ok(());
        }

        state.origin_x = bounds.origin.x;
        state.origin_y = bounds.origin.y;
        state.caret_x += dx;
        state.caret_y += dy;

        let caret_position = LogicalPosition::new(state.caret_x, state.caret_y).into();
        let caret_size = LogicalSize::new(state.caret_w, state.caret_h).into();
        drop(follow);

        debug!(dx, dy, "Following terminal window move");
        Self::send_caret_geometry(caret_position, caret_size);
        Ok(())
    }

    #[allow(clippy::unused_self)]
    pub(super) fn position_window(
        &self,
        webview_window: &tao::window::Window,
        _window_id: &WindowId,
        position: Position,
    ) -> wry::Result<()> {
        webview_window.set_outer_position(position);
        std::result::Result::Ok(())
    }

    #[allow(clippy::unused_self)]
    pub(super) fn get_cursor_position(&self) -> Option<Rect> {
        let caret: CaretPosition = unsafe { get_caret_position(true) };

        if caret.valid {
            Some(Rect {
                position: LogicalPosition::new(caret.x, caret.y).into(),
                size: LogicalSize::new(DEFAULT_CARET_WIDTH, caret.height).into(),
            })
        } else {
            None
        }
    }

    /// Gets the currently active window on the platform
    pub(super) fn get_active_window(&self) -> Option<PlatformWindow> {
        let active_window = self.focused_window.lock().unwrap().as_ref()?.clone();
        Some(PlatformWindow {
            rect: active_window.get_bounds()?.into(),
            inner: active_window,
        })
    }

    pub(super) async fn icon_lookup(asset: &AssetSpecifier<'_>) -> Option<ProcessedAsset> {
        let data = match asset {
            AssetSpecifier::Named(name) => unsafe { macos_utils::image::png_for_name(name)? },
            AssetSpecifier::PathBased(path) => (unsafe { macos_utils::image::png_for_path(path) })
                .or_else(|| {
                    if path.to_str()?.ends_with('/') {
                        // /bin will always exist and looks like the default folder
                        // TODO: replace with `iconForContentType`
                        unsafe { macos_utils::image::png_for_path(Path::new("/bin")) }
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    if let Some(ext) = path.extension() {
                        unsafe { macos_utils::image::png_for_name(ext.to_str()?) }
                    } else {
                        None
                    }
                })
                .or_else(|| unsafe { macos_utils::image::png_for_name("file") })?,
        };

        Some((Arc::new(data.into()), AssetKind::Png))
    }

    pub(super) fn accessibility_is_enabled() -> Option<bool> {
        Some(ACCESSIBILITY_ENABLED.load(Ordering::SeqCst))
    }
}

pub const fn autocomplete_active() -> bool {
    true
}
