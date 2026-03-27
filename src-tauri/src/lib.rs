use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem, MenuItemKind};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::webview::{DownloadEvent, NewWindowResponse, WebviewWindowBuilder};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WindowEvent, Wry};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::{AppHandleExt as _, StateFlags, WindowExt as _};

#[cfg(desktop)]
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use url::Url;

const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ID: &str = "main-tray";
const MENU_TOGGLE_WINDOW: &str = "toggle-window";
const MENU_AUTOSTART: &str = "autostart";
const MENU_QUIT: &str = "quit";
const WHATSAPP_WEB_URL: &str = "https://web.whatsapp.com";
const WHATSAPP_BUSINESS_WEB_URL: &str = "https://business.web.whatsapp.com";
const AUTOSTART_ARG: &str = "--autostart";
const EXTERNAL_OPEN_SCHEME: &str = "wpp-external";
const LINK_INTERCEPT_INIT_SCRIPT: &str = r#"
(() => {
  if (window.__WPP_TAURI_LINK_PATCH__) {
    return;
  }

  window.__WPP_TAURI_LINK_PATCH__ = true;

  const INTERNAL_ORIGINS = new Set([
    "https://web.whatsapp.com",
    "https://business.web.whatsapp.com"
  ]);
  const EXTERNAL_SCHEMES = new Set(["http:", "https:", "mailto:", "tel:"]);

  const normalizeExternalTarget = (value) => {
    try {
      const url = new URL(value, window.location.href);
      if (INTERNAL_ORIGINS.has(url.origin)) {
        return null;
      }
      if (!EXTERNAL_SCHEMES.has(url.protocol)) {
        return null;
      }
      return url.href;
    } catch (_) {
      return null;
    }
  };

  const requestExternalOpen = (value) => {
    const target = normalizeExternalTarget(value);
    if (!target) {
      return false;
    }

    window.location.href = "wpp-external://open?target=" + encodeURIComponent(target);
    return true;
  };

  document.addEventListener(
    "click",
    (event) => {
      if (event.defaultPrevented || event.button !== 0) {
        return;
      }
      if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
      }

      const originTarget = event.target;
      if (!(originTarget instanceof Element)) {
        return;
      }

      const anchor = originTarget.closest("a[href]");
      if (!anchor) {
        return;
      }

      if (requestExternalOpen(anchor.href)) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
    true
  );

  const nativeWindowOpen = window.open.bind(window);
  window.open = function (url, target, features) {
    if (typeof url === "string" && requestExternalOpen(url)) {
      return null;
    }

    return nativeWindowOpen(url, target, features);
  };

  const createNotificationPermissionStatus = () => ({
    state:
      window.Notification?.permission === "granted"
        ? "granted"
        : window.Notification?.permission === "denied"
          ? "denied"
          : "prompt",
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    dispatchEvent() {
      return true;
    }
  });

  try {
    if (navigator.permissions?.query) {
      const nativePermissionsQuery = navigator.permissions.query.bind(navigator.permissions);
      navigator.permissions.query = async (descriptor) => {
        if (descriptor?.name === "notifications") {
          return createNotificationPermissionStatus();
        }

        return nativePermissionsQuery(descriptor);
      };
    }
  } catch (_) {}

  setTimeout(async () => {
    try {
      if (typeof window.Notification !== "function") {
        return;
      }

      if (window.Notification.permission !== "granted") {
        await window.Notification.requestPermission();
      }
    } catch (_) {}
  }, 0);
})();
"#;

struct ManagedState {
    restore_window_state_on_first_show: Mutex<bool>,
    toggle_window_item: MenuItem<Wry>,
    autostart_item: CheckMenuItem<Wry>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let start_hidden = std::env::args().any(|arg| arg == AUTOSTART_ARG);

            #[cfg(desktop)]
            {
                app.handle().plugin(tauri_plugin_autostart::init(
                    MacosLauncher::LaunchAgent,
                    Some(vec![AUTOSTART_ARG]),
                ))?;

                let mut window_state_builder = tauri_plugin_window_state::Builder::default();
                if start_hidden {
                    window_state_builder = window_state_builder.skip_initial_state(MAIN_WINDOW_LABEL);
                }
                app.handle().plugin(window_state_builder.build())?;
            }

            let tray_menu = MenuBuilder::new(app)
                .text(
                    MENU_TOGGLE_WINDOW,
                    if start_hidden {
                        "Abrir WhatsApp"
                    } else {
                        "Ocultar janela"
                    },
                )
                .check(MENU_AUTOSTART, "Iniciar com o sistema")
                .separator()
                .text(MENU_QUIT, "Sair")
                .build()?;

            let toggle_window_item = match tray_menu.get(MENU_TOGGLE_WINDOW) {
                Some(MenuItemKind::MenuItem(item)) => item,
                _ => unreachable!("missing tray toggle item"),
            };
            let autostart_item = match tray_menu.get(MENU_AUTOSTART) {
                Some(MenuItemKind::Check(item)) => item,
                _ => unreachable!("missing tray autostart item"),
            };

            #[cfg(desktop)]
            autostart_item.set_checked(app.autolaunch().is_enabled().unwrap_or(false))?;

            app.manage(ManagedState {
                restore_window_state_on_first_show: Mutex::new(start_hidden),
                toggle_window_item,
                autostart_item,
            });

            let mut tray_builder = tauri::tray::TrayIconBuilder::with_id(TRAY_ID)
                .menu(&tray_menu)
                .tooltip("WhatsApp")
                .show_menu_on_left_click(false);

            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }

            tray_builder.build(app)?;

            let app_handle = app.handle().clone();
            app_handle.on_menu_event(|app, event| match event.id.as_ref() {
                MENU_TOGGLE_WINDOW => toggle_main_window(app),
                MENU_AUTOSTART => toggle_autostart(app),
                MENU_QUIT => quit_app(app),
                _ => {}
            });
            app_handle.on_tray_icon_event(|app, event| match event {
                TrayIconEvent::Click {
                    id,
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } if id.as_ref() == TRAY_ID => toggle_main_window(app),
                TrayIconEvent::DoubleClick {
                    id,
                    button: MouseButton::Left,
                    ..
                } if id.as_ref() == TRAY_ID => show_main_window(app),
                _ => {}
            });

            let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyW);
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |app, active_shortcut, event| {
                        if event.state == ShortcutState::Pressed
                            && active_shortcut
                                .matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyW)
                        {
                            toggle_main_window(app);
                        }
                    })
                    .build(),
            )?;
            if let Err(error) = app.global_shortcut().register(shortcut) {
                eprintln!("failed to register global shortcut: {error}");
            }

            let navigation_app = app.handle().clone();
            let new_window_app = app.handle().clone();
            let main_window = WebviewWindowBuilder::new(
                app,
                MAIN_WINDOW_LABEL,
                WebviewUrl::External(WHATSAPP_WEB_URL.parse().unwrap()),
            )
            .title("WhatsApp")
            .inner_size(1280.0, 900.0)
            .min_inner_size(1000.0, 700.0)
            .resizable(true)
            .initialization_script(LINK_INTERCEPT_INIT_SCRIPT)
            .visible(!start_hidden)
            .focused(!start_hidden)
            .on_navigation(move |url| handle_navigation(&navigation_app, url.as_str()))
            .on_new_window(move |url, _| {
                if should_open_in_browser(url.as_str()) {
                    if let Err(error) = open_in_default_browser(&new_window_app, url.as_str()) {
                        eprintln!("failed to open external link: {error}");
                    }
                    NewWindowResponse::Deny
                } else {
                    NewWindowResponse::Allow
                }
            })
            .on_document_title_changed(|window, title| {
                handle_document_title_change(&window, &title);
            })
            .on_download(|webview, event| {
                match event {
                    DownloadEvent::Requested { destination, .. } => {
                        let mut dialog = webview
                            .app_handle()
                            .dialog()
                            .file()
                            .set_title("Salvar arquivo");

                        if let Some(parent) = destination.parent() {
                            dialog = dialog.set_directory(parent);
                        }

                        if let Some(file_name) =
                            destination.file_name().and_then(|name| name.to_str())
                        {
                            dialog = dialog.set_file_name(file_name);
                        }

                        let chosen_destination = dialog.blocking_save_file();

                        if let Some(file_path) = chosen_destination {
                            if let Ok(path) = file_path.into_path() {
                                *destination = path;
                            }
                        }
                    }
                    DownloadEvent::Finished { url, path, success } => {
                        println!(
                            "Download finalizado | url={} | success={} | path={:?}",
                            url, success, path
                        );
                    }
                    _ => {}
                }

                true
            })
            .build()?;

            let window_app = app.handle().clone();
            main_window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    hide_main_window(&window_app);
                }
            });

            ensure_notification_permission(app.handle());
            sync_window_controls(app.handle());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn sync_window_controls(app: &AppHandle<Wry>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };

    let is_open = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    let state = app.state::<ManagedState>();
    let next_label = if is_open {
        "Ocultar janela"
    } else {
        "Abrir WhatsApp"
    };
    let _ = state.toggle_window_item.set_text(next_label);

    #[cfg(desktop)]
    {
        let _ = state
            .autostart_item
            .set_checked(app.autolaunch().is_enabled().unwrap_or(false));
    }
}

fn maybe_restore_window_state(window: &WebviewWindow<Wry>) {
    let app = window.app_handle();
    let state = app.state::<ManagedState>();
    let mut should_restore = state
        .restore_window_state_on_first_show
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if *should_restore {
        let _ = window.restore_state(StateFlags::all());
        *should_restore = false;
    }
}

fn show_main_window(app: &AppHandle<Wry>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };

    maybe_restore_window_state(&window);
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    sync_window_controls(app);
}

fn hide_main_window(app: &AppHandle<Wry>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };

    let _ = window.hide();
    sync_window_controls(app);
}

fn toggle_main_window(app: &AppHandle<Wry>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };

    let should_open = !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false);
    if should_open {
        show_main_window(app);
    } else {
        hide_main_window(app);
    }
}

fn quit_app(app: &AppHandle<Wry>) {
    let _ = app.save_window_state(StateFlags::all());
    app.exit(0);
}

fn toggle_autostart(app: &AppHandle<Wry>) {
    #[cfg(desktop)]
    {
        let autostart_manager = app.autolaunch();
        let enabled = autostart_manager.is_enabled().unwrap_or(false);
        let result = if enabled {
            autostart_manager.disable()
        } else {
            autostart_manager.enable()
        };

        if let Err(error) = result {
            eprintln!("failed to toggle autostart: {error}");
        }
    }

    sync_window_controls(app);
}

fn is_internal_whatsapp_url(url: &str) -> bool {
    url.starts_with(WHATSAPP_WEB_URL) || url.starts_with(WHATSAPP_BUSINESS_WEB_URL)
}

fn should_open_in_browser(url: &str) -> bool {
    if is_internal_whatsapp_url(url) {
        return false;
    }

    matches!(
        url,
        value
            if value.starts_with("http://")
                || value.starts_with("https://")
                || value.starts_with("mailto:")
                || value.starts_with("tel:")
    )
}

fn extract_external_open_target(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != EXTERNAL_OPEN_SCHEME {
        return None;
    }

    parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "target").then(|| value.into_owned()))
}

fn open_in_default_browser(
    app: &AppHandle<Wry>,
    url: &str,
) -> Result<(), tauri_plugin_opener::Error> {
    app.opener().open_url(url, None::<&str>)
}

fn handle_navigation(app: &AppHandle<Wry>, url: &str) -> bool {
    if let Some(target) = extract_external_open_target(url) {
        if let Err(error) = open_in_default_browser(app, &target) {
            eprintln!("failed to open external request: {error}");
        }
        return false;
    }

    if should_open_in_browser(url) {
        if let Err(error) = open_in_default_browser(app, url) {
            eprintln!("failed to open external navigation: {error}");
        }
        false
    } else {
        true
    }
}

fn ensure_notification_permission(app: &AppHandle<Wry>) {
    match app.notification().permission_state() {
        Ok(PermissionState::Prompt | PermissionState::PromptWithRationale) => {
            let _ = app.notification().request_permission();
        }
        Ok(PermissionState::Granted | PermissionState::Denied) => {}
        Err(error) => eprintln!("failed to check notification permission: {error}"),
    }
}

fn handle_document_title_change(window: &WebviewWindow<Wry>, title: &str) {
    let next_title = if title.is_empty() { "WhatsApp" } else { title };
    let _ = window.set_title(next_title);
}
