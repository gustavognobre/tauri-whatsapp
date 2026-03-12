use tauri::webview::{DownloadEvent, WebviewWindowBuilder};
use tauri::{Manager, WebviewUrl};
use tauri_plugin_dialog::DialogExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External("https://web.whatsapp.com".parse().unwrap()),
            )
            .title("WhatsApp")
            .inner_size(1280.0, 900.0)
            .min_inner_size(1000.0, 700.0)
            .resizable(true)
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

                        if let Some(file_name) = destination.file_name().and_then(|name| name.to_str()) {
                            dialog = dialog.set_file_name(file_name);
                        }

                        let destino_escolhido = dialog.blocking_save_file();

                        if let Some(file_path) = destino_escolhido {
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
