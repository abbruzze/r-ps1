use tracing_subscriber;
use tracing_subscriber::{fmt, reload, EnvFilter, Registry};
use tracing_subscriber::prelude::*;

pub struct Logger {
    reload_handle: reload::Handle<EnvFilter, Registry>,
}

impl Logger {
    pub fn new(initial_level:&str) -> Self {
        let filter = EnvFilter::new(initial_level);
        let (filter_layer, reload_handle) = reload::Layer::new(filter);

        tracing_subscriber::registry()
            .with(filter_layer)
            .with(fmt::layer())
            .init();

        Self { reload_handle }
    }

    pub fn set_log_level(&self, level: &str) {
        self.reload_handle
            .modify(|f| *f = EnvFilter::new(level))
            .unwrap();
    }
}

pub fn init_logging() {
    let filter = EnvFilter::new("info");
    let (filter_layer, reload_handle) = reload::Layer::new(filter);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt::layer())
        .init();
    /*
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        //.with_env_filter(EnvFilter::from_default_env())
        .with_timer(fmt::time::Uptime::default()) // timestamp leggibile
        .with_level(true)
        //.with_thread_ids(true)
        //.with_thread_names(true)
        .init();

     */
}