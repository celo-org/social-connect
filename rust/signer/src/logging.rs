//! Log output setup.

use std::io;

use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, Layer};

use crate::config::LogFormat;

/// Installs the global log subscriber, writing to stdout in the given format.
///
/// `RUST_LOG` selects what is logged; without it the level is `info`.
pub fn init(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let registry = tracing_subscriber::registry().with(filter);
    match format {
        LogFormat::Text => registry.with(tracing_subscriber::fmt::layer()).init(),
        LogFormat::Json => registry.with(json_layer(io::stdout)).init(),
    }
}

/// One JSON object per line with `timestamp`, `level`, `target`, `message` and
/// the event's own fields at the top level, so log collectors can filter on
/// them without unnesting. The enclosing span is kept; the full span list is
/// dropped because it repeats it.
fn json_layer<S, W>(writer: W) -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_span_list(false)
        .with_writer(writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct CapturedOutput(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CapturedOutput {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn json_lines_carry_level_message_and_fields_at_the_top_level() {
        let output = CapturedOutput::default();
        let writer = output.clone();
        let subscriber = tracing_subscriber::registry().with(json_layer(move || writer.clone()));

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("request", endpoint = "/sign");
            let _entered = span.enter();
            tracing::warn!(account = "0xabc", performed = 3, "quota exceeded");
        });

        let bytes = output.0.lock().unwrap().clone();
        let text = String::from_utf8(bytes).unwrap();
        let mut lines = text.lines();
        let line: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert!(lines.next().is_none(), "one event must give one line");

        assert_eq!(line["level"], "WARN");
        assert_eq!(line["message"], "quota exceeded");
        assert_eq!(line["account"], "0xabc");
        assert_eq!(line["performed"], 3);
        assert_eq!(line["span"]["endpoint"], "/sign");
        assert!(line["timestamp"].is_string());
        assert!(line["target"].as_str().unwrap().ends_with("logging::tests"));
        assert!(line.get("spans").is_none());
        assert!(line.get("fields").is_none());
    }
}
