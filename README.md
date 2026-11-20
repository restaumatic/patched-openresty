# patched-openresty

This is a customized version of OpenResty that includes changes in the nginx source.

To make changes easier, this repository is not structured like the upstream OpenResty repo.
Instead, it's an openresty release bundle unpacked into a Git repository.

## Applied changes

Added a metrics system.

The included metrics are:

- `any_event_handler_time_ns`: Time spent in each event handler call in the event loop
- `event_handler_{total,self}_{wall,cpu}_time_ns{handler="<name>"}`: The same per handler, for handlers listed in `<nginx>.symbols` (from `generate-symbols`). Self time excludes nested handler calls.
- `open_and_stat_file_time_ns`: Time spent in `ngx_open_and_stat_file` function
- `event_loop_latency_ns`: Event loop iteration time (from `epoll_wait` return to next call to `epoll_wait`)

The metrics can be read from Lua using FFI. There's no lua api provided here.

See `ngx_metrics.h` and functions `ngx_metrics_prometheus` and `ngx_metrics_free_result`.

Metrics are kept per worker process, and a read returns the numbers of the worker that served it, so they are meaningful only with `worker_processes 1`.
