#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write;

use crate::ffi::{cpu_time_ns, wall_time_ns};

/// In the order of `ngx_metric_e`.
pub const BUILTIN_NAMES: [&str; 3] =
    ["any_event_handler_time_ns", "open_and_stat_file_time_ns", "event_loop_latency_ns"];
const ANY_EVENT_HANDLER_TIME_NS: usize = 0;

const MAX_DYNAMIC_METRICS: usize = 128 * 4;

const SPAN_METRIC_NAMES: [&str; 4] = [
    "event_handler_total_wall_time_ns",
    "event_handler_self_wall_time_ns",
    "event_handler_total_cpu_time_ns",
    "event_handler_self_cpu_time_ns",
];

#[derive(Default)]
struct Metric {
    name: &'static str,
    tags: String,
    count: i64,
    sum: i64,
    min: i64,
    max: i64,
}

impl Metric {
    fn report(&mut self, value: i64) {
        if self.count == 0 {
            self.min = value;
            self.max = value;
        } else {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
        self.count += 1;
        self.sum += value;
    }

    fn write_prometheus(&self, out: &mut String) {
        let Metric { name, tags, count, sum, min, max } = self;
        let (avg, min, max) = match count {
            0 => (0.0, 0, 0),
            _ => (*sum as f64 / *count as f64, *min, *max),
        };
        let _ = write!(
            out,
            "{name}_count{tags} {count}\n{name}_sum{tags} {sum}\n{name}_avg{tags} {avg}\n\
             {name}_min{tags} {min}\n{name}_max{tags} {max}\n"
        );
    }
}

struct Handler {
    tags: String,
    metrics: Option<[usize; 4]>,
}

struct Span {
    handler: usize,
    start_wall: i64,
    start_cpu: i64,
    children_wall: i64,
    children_cpu: i64,
}

struct State {
    base_address: usize,
    handlers: HashMap<usize, Handler>,
    /// The builtins at their `ngx_metric_e` index, then per-handler metrics.
    metrics: Vec<Metric>,
    spans: Vec<Span>,
}

// Set by `init` on nginx's main thread (inherited by forked workers).
// Other threads see `None`, so their calls are ignored.
thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Returns `None` off the main thread, on re-entry, or during thread teardown.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE
        .try_with(|cell| {
            let mut state = cell.try_borrow_mut().ok()?;
            state.as_mut().map(f)
        })
        .ok()
        .flatten()
}

impl State {
    fn new(base_address: usize, symbols: HashMap<usize, String>) -> Self {
        let handlers = symbols
            .into_iter()
            .filter(|(_, name)| !name.contains('\0'))
            .map(|(addr, name)| (addr, Handler { tags: format!("{{handler=\"{name}\"}}"), metrics: None }))
            .collect();
        let mut metrics = Vec::with_capacity(BUILTIN_NAMES.len() + MAX_DYNAMIC_METRICS);
        metrics.extend(BUILTIN_NAMES.map(|name| Metric { name, ..Default::default() }));
        State { base_address, handlers, metrics, spans: Vec::new() }
    }

    fn enter_span(&mut self, handler: usize) {
        self.spans.push(Span {
            handler,
            start_wall: wall_time_ns(),
            start_cpu: cpu_time_ns(),
            children_wall: 0,
            children_cpu: 0,
        });
    }

    fn exit_span(&mut self) {
        let Some(span) = self.spans.pop() else { return };
        let wall = wall_time_ns() - span.start_wall;
        let cpu = cpu_time_ns() - span.start_cpu;
        if let Some(parent) = self.spans.last_mut() {
            parent.children_wall += wall;
            parent.children_cpu += cpu;
        }
        self.metrics[ANY_EVENT_HANDLER_TIME_NS].report(wall);
        if let Some(relative_addr) = span.handler.checked_sub(self.base_address) {
            self.report_handler(relative_addr, [wall, wall - span.children_wall, cpu, cpu - span.children_cpu]);
        }
    }

    fn report_handler(&mut self, relative_addr: usize, values: [i64; 4]) {
        let Some(handler) = self.handlers.get_mut(&relative_addr) else { return };
        let ids = match handler.metrics {
            Some(ids) => ids,
            None => {
                let dynamic = self.metrics.len() - BUILTIN_NAMES.len();
                if MAX_DYNAMIC_METRICS - dynamic < SPAN_METRIC_NAMES.len() {
                    return;
                }
                let ids = SPAN_METRIC_NAMES.map(|name| {
                    self.metrics.push(Metric { name, tags: handler.tags.clone(), ..Default::default() });
                    self.metrics.len() - 1
                });
                *handler.metrics.insert(ids)
            }
        };
        for (id, value) in ids.into_iter().zip(values) {
            self.metrics[id].report(value);
        }
    }

    fn prometheus(&mut self, reset: bool) -> String {
        let mut out = String::new();
        for m in &mut self.metrics {
            m.write_prometheus(&mut out);
            if reset {
                *m = Metric { name: m.name, tags: std::mem::take(&mut m.tags), ..Default::default() };
            }
        }
        out
    }
}

/// Only the first call on a thread has an effect.
pub fn init(base_address: usize, symbols: HashMap<usize, String>) {
    let _ = STATE.try_with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            if state.is_none() {
                *state = Some(State::new(base_address, symbols));
            }
        }
    });
}

/// Ignores an index that isn't a builtin.
pub fn report_builtin(index: usize, value: i64) {
    with_state(|s| {
        if index < BUILTIN_NAMES.len() {
            s.metrics[index].report(value);
        }
    });
}

pub fn prometheus(reset: bool) -> Option<String> {
    with_state(|s| s.prometheus(reset))
}

pub fn enter_span(handler: usize) {
    with_state(|s| s.enter_span(handler));
}

pub fn exit_span() {
    with_state(State::exit_span);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(symbols: &[(usize, &str)]) -> State {
        State::new(0x1000, symbols.iter().map(|&(addr, name)| (addr, name.to_string())).collect())
    }

    #[test]
    fn reentry_and_other_threads_are_ignored() {
        STATE.set(Some(test_state(&[])));
        assert_eq!(with_state(|_| with_state(|_| ())), Some(None));
        assert_eq!(std::thread::spawn(|| with_state(|_| ())).join().unwrap(), None);
        assert_eq!(with_state(|_| ()), Some(()));
    }

    #[test]
    fn handlers_beyond_capacity_get_no_metrics() {
        let names: Vec<_> = (0..=MAX_DYNAMIC_METRICS / 4).map(|i| (i, format!("h{i}"))).collect();
        let mut s = test_state(&names.iter().map(|(a, n)| (*a, n.as_str())).collect::<Vec<_>>());
        for (addr, _) in &names {
            s.report_handler(*addr, [1; 4]);
        }
        assert_eq!(s.metrics.len(), BUILTIN_NAMES.len() + MAX_DYNAMIC_METRICS);
        assert_eq!(s.handlers.values().filter(|h| h.metrics.is_none()).count(), 1);
    }

    #[test]
    fn spans_report_self_and_total_time() {
        let mut s = test_state(&[(0x10, "outer"), (0x20, "inner")]);
        s.enter_span(0x1010);
        s.enter_span(0x1020);
        s.exit_span();
        s.exit_span();
        assert_eq!(s.metrics.len(), 3 + 8);
        assert_eq!(s.metrics[3].tags, "{handler=\"inner\"}");
        let [inner_total, _, _, _] = [3, 4, 5, 6].map(|i| s.metrics[i].sum);
        let [outer_total, outer_self, _, _] = [7, 8, 9, 10].map(|i| s.metrics[i].sum);
        assert_eq!(outer_self, outer_total - inner_total);
        assert_eq!(s.metrics[ANY_EVENT_HANDLER_TIME_NS].count, 2);
        assert!(s.metrics[3..].iter().all(|m| m.count == 1));
        assert!(s.spans.is_empty());
    }

    #[test]
    fn prometheus_renders_and_resets() {
        let mut s = test_state(&[(0x10, "h")]);
        s.metrics[2].report(10);
        s.metrics[2].report(5);
        s.report_handler(0x10, [3; 4]);
        let out = s.prometheus(true);
        assert!(out.starts_with(
            "any_event_handler_time_ns_count 0\nany_event_handler_time_ns_sum 0\n\
             any_event_handler_time_ns_avg 0\nany_event_handler_time_ns_min 0\nany_event_handler_time_ns_max 0\n"
        ));
        assert!(out.contains(
            "event_loop_latency_ns_count 2\nevent_loop_latency_ns_sum 15\nevent_loop_latency_ns_avg 7.5\n\
             event_loop_latency_ns_min 5\nevent_loop_latency_ns_max 10\n"
        ));
        assert!(out.contains("event_handler_self_cpu_time_ns_max{handler=\"h\"} 3\n"));
        assert_eq!(out.lines().count(), (3 + 4) * 5);
        assert!(s.metrics.iter().all(|m| m.count == 0 && m.sum == 0));
        assert!(s.prometheus(false).contains("event_handler_self_cpu_time_ns_count{handler=\"h\"} 0\n"));
    }
}
