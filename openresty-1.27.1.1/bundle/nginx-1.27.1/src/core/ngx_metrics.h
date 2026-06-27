#ifndef _NGX_METRICS_H_INCLUDED_
#define _NGX_METRICS_H_INCLUDED_

#include <stdint.h>

typedef struct {
  const char *name;
  const char *tags;

  int64_t count;
  int64_t sum;
  int64_t min;
  int64_t max;
} ngx_metric_t;

struct ngx_log_s;
typedef struct ngx_log_s             ngx_log_t;

void ngx_metrics_init(ngx_log_t *log);

int ngx_get_num_metrics();
ngx_metric_t *ngx_get_metric(int index);

extern ngx_metric_t ngx_metric_any_event_handler_time_ns;
extern ngx_metric_t ngx_metric_open_and_stat_file_time_ns;
extern ngx_metric_t ngx_metric_event_loop_latency_ns;

// Report a measurement for a given metric.
void ngx_metric_report(ngx_metric_t *metric, int64_t value);

#define NGX_CALL_HANDLER(handler, ...) do { ngx_span_t __span; ngx_metrics_span_enter(&__span, handler); (handler)(__VA_ARGS__); ngx_metrics_span_exit(&__span); } while(0)

typedef struct ngx_span_s {
  struct ngx_span_s *parent;
  void *handler;
  int64_t start_time;
  int64_t children_time;
} ngx_span_t;

void ngx_metrics_span_enter(ngx_span_t *span, void *handler);
void ngx_metrics_span_exit(ngx_span_t *span);

int64_t ngx_precise_time();

struct lua_State;
typedef struct lua_State lua_State;

int ngx_metrics_dump_lua_stack(lua_State *L, char *buf, int bufsize);

#endif /* _NGX_METRICS_H_INCLUDED_ */
