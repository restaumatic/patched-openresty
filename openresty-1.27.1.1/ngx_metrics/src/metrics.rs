#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::ffi::{c_char, CStr, CString};

use crate::ffi::{cpu_time_ns, ngx_metric_t, wall_time_ns};

const MAX_DYNAMIC_METRICS: usize = 128 * 4;

const SPAN_METRIC_NAMES: [&CStr; 4] = [
    c"event_handler_total_wall_time_ns",
    c"event_handler_self_wall_time_ns",
    c"event_handler_total_cpu_time_ns",
    c"event_handler_self_cpu_time_ns",
];

impl ngx_metric_t {
    pub const fn new(name: &'static CStr, tags: *const c_char) -> Self {
        Self { name: name.as_ptr(), tags, count: 0, sum: 0, min: 0, max: 0 }
    }

    pub fn report(&mut self, value: i64) {
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
}

/// A vector with a fixed capacity, allocated up front, so pointers to its elements stay valid.
/// Could be an `arrayvec::ArrayVec`; hand-rolled to avoid the dependency.
struct FixedVec<T> {
    items: Box<[T]>,
    len: usize,
}

impl<T: Clone> FixedVec<T> {
    /// `fill` takes up the unused slots, so none of them is uninitialized.
    fn new(capacity: usize, fill: T) -> Self {
        Self { items: vec![fill; capacity].into_boxed_slice(), len: 0 }
    }

    fn remaining_capacity(&self) -> usize {
        self.items.len() - self.len
    }

    /// Returns the new element's index, or gives the element back when full.
    fn try_push(&mut self, item: T) -> Result<usize, T> {
        let Some(slot) = self.items.get_mut(self.len) else { return Err(item) };
        *slot = item;
        self.len += 1;
        Ok(self.len - 1)
    }
}

impl<T> Deref for FixedVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.items[..self.len]
    }
}

impl<T> DerefMut for FixedVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.items[..self.len]
    }
}

struct Handler {
    tags: CString,
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
    dynamic: FixedVec<ngx_metric_t>,
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
            .filter_map(|(addr, name)| {
                let tags = CString::new(format!("{{handler=\"{name}\"}}")).ok()?;
                Some((addr, Handler { tags, metrics: None }))
            })
            .collect();
        State { base_address, handlers, dynamic: FixedVec::new(MAX_DYNAMIC_METRICS, ngx_metric_t::new(c"", std::ptr::null())), spans: Vec::new() }
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

    /// Returns the span's total wall time.
    fn exit_span(&mut self) -> Option<i64> {
        let span = self.spans.pop()?;
        let wall = wall_time_ns() - span.start_wall;
        let cpu = cpu_time_ns() - span.start_cpu;
        if let Some(parent) = self.spans.last_mut() {
            parent.children_wall += wall;
            parent.children_cpu += cpu;
        }
        self.report_handler(
            span.handler.wrapping_sub(self.base_address),
            [wall, wall - span.children_wall, cpu, cpu - span.children_cpu],
        );
        Some(wall)
    }

    fn report_handler(&mut self, relative_addr: usize, values: [i64; 4]) {
        let Some(handler) = self.handlers.get_mut(&relative_addr) else { return };
        let ids = match handler.metrics {
            Some(ids) => ids,
            None => {
                // All four or none, so a handler never has some of its metrics.
                if self.dynamic.remaining_capacity() < SPAN_METRIC_NAMES.len() {
                    return;
                }
                let mut ids = [0; 4];
                for (id, name) in ids.iter_mut().zip(SPAN_METRIC_NAMES) {
                    let Ok(i) = self.dynamic.try_push(ngx_metric_t::new(name, handler.tags.as_ptr())) else {
                        return;
                    };
                    *id = i;
                }
                *handler.metrics.insert(ids)
            }
        };
        for (id, value) in ids.into_iter().zip(values) {
            self.dynamic[id].report(value);
        }
    }
}

/// Only the first call on a thread has an effect: replacing the state would invalidate the pointers
/// `dynamic` has handed out.
pub fn init(base_address: usize, symbols: HashMap<usize, String>) {
    let _ = STATE.try_with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            if state.is_none() {
                *state = Some(State::new(base_address, symbols));
            }
        }
    });
}

/// False off the main thread, before `init` and while the state is borrowed.
pub fn active() -> bool {
    with_state(|_| ()).is_some()
}

pub fn num_dynamic() -> usize {
    with_state(|s| s.dynamic.len()).unwrap_or(0)
}

pub fn dynamic(index: usize) -> Option<*mut ngx_metric_t> {
    with_state(|s| s.dynamic.get_mut(index).map(|m| m as *mut _)).flatten()
}

pub fn enter_span(handler: usize) {
    with_state(|s| s.enter_span(handler));
}

/// Returns the span's total wall time.
pub fn exit_span() -> Option<i64> {
    with_state(State::exit_span).flatten()
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
        assert_eq!(s.dynamic.remaining_capacity(), 0);
        assert_eq!(s.handlers.values().filter(|h| h.metrics.is_none()).count(), 1);
    }

    #[test]
    fn spans_report_self_and_total_time() {
        let mut s = test_state(&[(0x10, "outer"), (0x20, "inner")]);
        s.enter_span(0x1010);
        s.enter_span(0x1020);
        s.exit_span();
        s.exit_span();
        assert_eq!(s.dynamic.len(), 8);
        assert_eq!(
            CStr::from_bytes_with_nul(s.handlers[&0x20].tags.as_bytes_with_nul()).unwrap(),
            c"{handler=\"inner\"}"
        );
        let [inner_total, _, _, _] = [0, 1, 2, 3].map(|i| s.dynamic[i].sum);
        let [outer_total, outer_self, _, _] = [4, 5, 6, 7].map(|i| s.dynamic[i].sum);
        assert_eq!(outer_self, outer_total - inner_total);
        assert!(s.dynamic.iter().all(|m| m.count == 1));
        assert!(s.spans.is_empty());
    }
}
