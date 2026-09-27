//! Types and traits to enable interoperability between the other Blitz crates without
//! circular or unnecessary dependencies.

pub mod devtools;
pub mod events;
pub mod navigation;
pub mod net;
pub mod node_id;
pub mod shell;

pub use node_id::NodeId;
pub use smol_str::SmolStr;

pub fn probe_enabled() -> bool {
    static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        if std::env::var_os("BLITZ_DEBUG_PROBE").is_some() {
            ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn probe_log(tag: &str, msg: &str) {
    if !probe_enabled() {
        return;
    }
    let log_path = std::env::temp_dir().join("blitz-windows-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        use std::io::Write;
        let _ = writeln!(f, "[PROBE][{tag}] {msg}");
    }
}

#[macro_export]
macro_rules! probe {
    ($tag:expr, $($arg:tt)*) => {
        if $crate::probe_enabled() {
            $crate::probe_log($tag, &format!($($arg)*));
        }
    };
}

