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
void ngx_metric_report(ngx_metric_e metric, int64_t value);

// Returns all metrics in the Prometheus text format, to be freed with ngx_metrics_free.
// With `reset`, zeroes them afterwards. NULL off the main thread and before ngx_metrics_init.
// TODO: serve metrics without Lua, from an nginx directive (e.g. with the ngx crate).
char *ngx_metrics_prometheus(int reset);
void ngx_metrics_free(char *metrics);

// Calls `handler(...)` inside a span: see ngx_metrics_span_enter.
#define NGX_CALL_HANDLER(handler, ...) do { ngx_metrics_span_enter(handler); (handler)(__VA_ARGS__); ngx_metrics_span_exit(); } while(0)

// Opens a span for the function at `handler`. Spans nest; each enter needs a matching exit.
// On exit, the span's total wall time goes to NGX_METRIC_ANY_EVENT_HANDLER_TIME_NS. If `handler`
// is in the symbols file, its total and self (minus nested spans) wall and CPU times also go to the
// event_handler_* metrics tagged `{handler="<name>"}`.
// Ignored off the main thread and before ngx_metrics_init.
void ngx_metrics_span_enter(void *handler);
// Closes the innermost open span; does nothing if none is open.
void ngx_metrics_span_exit(void);

int64_t ngx_precise_time();

struct lua_State;
typedef struct lua_State lua_State;

int ngx_metrics_dump_lua_stack(lua_State *L, char *buf, int bufsize);

#endif /* _NGX_METRICS_H_INCLUDED_ */
