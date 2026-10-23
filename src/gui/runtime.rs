use std::sync::OnceLock;
use tokio::runtime::{Handle, Runtime};

static TOKIO: OnceLock<Runtime> = OnceLock::new();

/// A handle onto a tokio runtime that stays alive for the whole process.
///
/// `LogManager`/`FileReader` go through `sqlx`/`tokio::task::spawn_blocking`,
/// which need a live tokio reactor — but gpui's own async tasks
/// (`Context::spawn`) run on gpui's foreground executor, not tokio's. The
/// glue layer bridges the two by spawning the actual TUI-core async work
/// onto this handle and awaiting the resulting `JoinHandle` from inside a
/// gpui task.
pub fn handle() -> Handle {
    TOKIO
        .get_or_init(|| Runtime::new().expect("failed to start the shared tokio runtime"))
        .handle()
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_can_spawn_and_run_a_task() {
        let result = handle().block_on(handle().spawn(async { 1 + 1 })).unwrap();
        assert_eq!(result, 2);
    }

    #[test]
    fn repeated_calls_return_a_working_handle() {
        let first = handle();
        let second = handle();
        let result = second.block_on(first.spawn(async { "ok" })).unwrap();
        assert_eq!(result, "ok");
    }
}
