use std::path::PathBuf;
use tracing_subscriber;
use tracing_subscriber::{fmt, reload, EnvFilter, Registry};
use tracing_subscriber::prelude::*;

pub struct Logger {
    reload_handle: reload::Handle<EnvFilter, Registry>,
}

impl Logger {
    pub fn new(log_path: Option<PathBuf>, level: String) -> Self {
        // Crea filtro (es: "info", "debug", oppure "module=trace")
        let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));

        // Layer ricaricabile
        let (filter_layer, reload_handle) = reload::Layer::new(filter);

        // Writer (file o stdout)
        if let Some(path) = log_path {
            println!("Log file redirected to: {}",path.display());

            let file = std::fs::File::create(path).expect("Cannot create log file");

            let fmt_layer = fmt::layer().with_writer(file).with_ansi(false);

            let subscriber = Registry::default()
                .with(filter_layer)
                .with(fmt_layer);

            tracing::subscriber::set_global_default(subscriber).expect("Cannot init global log settings");
        } else {
            let fmt_layer = fmt::layer();

            let subscriber = Registry::default()
                .with(filter_layer)
                .with(fmt_layer);

            tracing::subscriber::set_global_default(subscriber).expect("Cannot init global log settings");
        }

        Self { reload_handle }
    }

    pub fn set_log_level(&self, level: &str) {
        self.reload_handle
            .modify(|f| *f = EnvFilter::new(level))
            .unwrap();
    }
}