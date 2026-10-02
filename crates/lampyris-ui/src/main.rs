//! Lampyris desktop app (Qt Quick + KDE Kirigami).

mod controller;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    // LAMPYRIS_QML=/path/to/main.qml loads QML from disk instead of the copy
    // compiled into the binary, so the interface can be iterated without rebuilding.
    let url = match std::env::var("LAMPYRIS_QML") {
        Ok(path) => format!(
            "file://{}",
            std::fs::canonicalize(&path)
                .unwrap_or(path.into())
                .display()
        ),
        Err(_) => "qrc:/qt/qml/org/bogd3v/lampyris/qml/main.qml".to_string(),
    };
    if let Some(mut engine) = engine.as_mut() {
        // Without this, a QML error (e.g. Kirigami not installed) would leave the
        // process running with no window.
        engine
            .as_mut()
            .on_object_creation_failed(|_, url| {
                eprintln!(
                    "lampyris-ui: could not load {url}. Is KDE Kirigami (KF6) installed? \
                     On Debian/Ubuntu: qml6-module-org-kde-kirigami; Fedora: kf6-kirigami; \
                     Arch: kirigami."
                );
                std::process::exit(1);
            })
            .release();
        engine.load(&QUrl::from(url.as_str()));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
