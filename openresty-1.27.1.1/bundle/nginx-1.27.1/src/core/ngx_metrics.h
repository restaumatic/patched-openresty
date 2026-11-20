#ifndef _NGX_METRICS_H_INCLUDED_
#define _NGX_METRICS_H_INCLUDED_

#include <stdint.h>

struct ngx_log_s;
typedef struct ngx_log_s             ngx_log_t;

void ngx_metrics_init(ngx_log_t *log);

typedef enum {
  NGX_METRIC_ANY_EVENT_HANDLER_TIME_NS,
  NGX_METRIC_OPEN_AND_STAT_FILE_TIME_NS,
  NGX_METRIC_EVENT_LOOP_LATENCY_NS,
} ngx_metric_e;

// Report a measurement for a given metric.
//
// Ignored off the main thread and before ngx_metrics_init.
void ngx_metric_report(ngx_metric_e metric, int64_t value);

// Returns all metrics in the Prometheus text format, to be freed with ngx_metrics_free_result.
// With `reset`, zeroes them afterwards.
//
// NULL off the main thread and before ngx_metrics_init.
//
// From a Lua handler:
//   local ffi = require "ffi"
//   ffi.cdef[[
//     char *ngx_metrics_prometheus(int reset);
//     void ngx_metrics_free_result(char *metrics);
//   ]]
//   local metrics = ffi.C.ngx_metrics_prometheus(1)
//   if metrics == nil then return ngx.exit(ngx.HTTP_SERVICE_UNAVAILABLE) end
//   ngx.print(ffi.string(ffi.gc(metrics, ffi.C.ngx_metrics_free_result)))
//
// TODO: serve metrics without Lua, from an nginx directive (e.g. with the ngx crate).
char *ngx_metrics_prometheus(int reset);
void ngx_metrics_free_result(char *metrics);

// Calls `handler(...)` inside a span: see ngx_metrics_span_enter.
#define NGX_CALL_HANDLER(handler, ...) do { ngx_metrics_span_enter(handler); (handler)(__VA_ARGS__); ngx_metrics_span_exit(); } while(0)

// Opens a span for `handler`: its wall and CPU time and call count are tracked until the matching
// ngx_metrics_span_exit. Spans nest.
//
// Ignored off the main thread and before ngx_metrics_init.
void ngx_metrics_span_enter(void *handler);
// Closes the innermost open span.
void ngx_metrics_span_exit(void);

int64_t ngx_precise_time();

struct lua_State;
typedef struct lua_State lua_State;

int ngx_metrics_dump_lua_stack(lua_State *L, char *buf, int bufsize);

#endif /* _NGX_METRICS_H_INCLUDED_ */
