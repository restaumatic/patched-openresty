# patched-openresty

This is a customized version of OpenResty that includes changes in the nginx source.

To make changes easier, this repository is not structured like the upstream OpenResty repo.
Instead, it's an openresty release bundle unpacked into a Git repository.

## Applied changes

Added a metrics system.

The included metrics are:

- `event_handler_time_ns`: Time spent in event handlers in the event loop
- `open_and_stat_file_time_ns`: Time spent in `ngx_open_and_stat_file` function
- `event_loop_latency_ns`: Event loop iteration time (from `epoll_wait` return to next call to `epoll_wait`)

The metrics can be read from Lua using FFI. There's no lua api provided here.

See `ngx_metrics.h` and functions `ngx_get_num_metrics` and `ngx_get_metric`.
