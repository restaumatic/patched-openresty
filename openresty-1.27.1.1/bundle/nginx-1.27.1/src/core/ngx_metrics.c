#define _GNU_SOURCE

#include <ngx_metrics.h>
#include <ngx_config.h>
#include <ngx_core.h>
#include <ngx_log.h>

#include <time.h>
#include <stdint.h>
#include <stdio.h>
#include <link.h>
#include <stdbool.h>
#include <stdlib.h>

#define METRIC_INIT(name) { name, NULL, 0, 0, 0, 0 }

typedef enum {
    SPAN_METRIC_TOTAL_WALL_TIME = 0,
    SPAN_METRIC_SELF_WALL_TIME,
    SPAN_METRIC_TOTAL_CPU_TIME,
    SPAN_METRIC_SELF_CPU_TIME,
} span_metric_type_e;

#define MAX_DYNAMIC_METRICS (128 * NGX_METRICS_NUM_SPAN_METRIC_TYPES)

static const char *METRIC_TYPE_NAMES[] = {
  "event_handler_total_wall_time_ns",
  "event_handler_self_wall_time_ns",
  "event_handler_total_cpu_time_ns",
  "event_handler_self_cpu_time_ns",
};

ngx_metric_t *get_metric_for_symbol(ngx_metrics_symbol_t *sym, span_metric_type_e metric_type);

// Summary metric for all event handlers.
// There's another metric - event_handler_time_ns - tagged by the handler name.
ngx_metric_t ngx_metric_any_event_handler_time_ns = METRIC_INIT("any_event_handler_time_ns");

ngx_metric_t ngx_metric_open_and_stat_file_time_ns = METRIC_INIT("open_and_stat_file_time_ns");
ngx_metric_t ngx_metric_event_loop_latency_ns = METRIC_INIT("event_loop_latency_ns");

#define NUM_BUILTIN_METRICS (sizeof(builtin_metrics) / sizeof(builtin_metrics[0]))

ngx_metric_t *builtin_metrics[] = {
  &ngx_metric_any_event_handler_time_ns,
  &ngx_metric_open_and_stat_file_time_ns,
  &ngx_metric_event_loop_latency_ns,
};

static ngx_metric_t dynamic_metrics[MAX_DYNAMIC_METRICS];
static int num_dynamic_metrics = 0;

void *get_base_address();

static void ngx_metrics_report_event_handler_time(void *handler, span_metric_type_e metric_type, int64_t value) {
  ngx_metric_report(&ngx_metric_any_event_handler_time_ns, value);

  uintptr_t relative_addr = (uintptr_t) handler - (uintptr_t) get_base_address();
  ngx_metrics_symbol_t *sym = ngx_metrics_find_symbol(relative_addr);
  /* fprintf(stderr, "event handler %p (%s) took %ld ms\n", */
  /*     relative_addr, */
  /*     sym ? sym->name : "unknown", */
  /*     value / 1000000); */
  ngx_metric_t *metric = sym ? get_metric_for_symbol(sym, metric_type) : NULL;
  if(metric) {
    /* fprintf(stderr, "tags: %d %s %d %p\n", getpid(), metric->tags, num_dynamic_metrics, metric->tags); */
    ngx_metric_report(metric, value);
  }
}

void
ngx_metric_report(ngx_metric_t *metric, int64_t value) {
  int64_t previous_count = metric->count++;
  metric->sum += value;

  if(previous_count == 0) {
    metric->min = value;
    metric->max = value;
  } else {
    if(value < metric->min) {
      metric->min = value;
    }
    if(value > metric->max) {
      metric->max = value;
    }
  }
}

int ngx_get_num_metrics() {
  return NUM_BUILTIN_METRICS + num_dynamic_metrics;
}

ngx_metric_t *ngx_get_metric(int index) {
  return (size_t)index < NUM_BUILTIN_METRICS ? builtin_metrics[index] : &dynamic_metrics[index - NUM_BUILTIN_METRICS];
}

void ngx_metrics_reset() {
  for(int i = 0; i < ngx_get_num_metrics(); i++) {
    ngx_metric_t *metric = ngx_get_metric(i);
    metric->count = 0;
    metric->sum = 0;
  }
}

int64_t ngx_precise_time() {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec * 1000000000L + ts.tv_nsec;
}

int64_t ngx_cpu_time() {
  struct timespec ts;
  clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts);
  return ts.tv_sec * 1000000000L + ts.tv_nsec;
}

static void *exe_base_address = 0;

static int callback(struct dl_phdr_info *info, size_t size, void *data) {
    if (info->dlpi_name[0] == '\0') { // The main executable has an empty name
        exe_base_address = (void *) info->dlpi_addr;
    }
    return 0;
}

void *get_base_address() {
  if(!exe_base_address) {
    dl_iterate_phdr(callback, NULL);
  }
  return exe_base_address;
}

void ngx_metrics_init(ngx_log_t *log) {
  char err[256];
  int num_symbols = ngx_metrics_load_symbols(err, sizeof(err));
  if(num_symbols < 0) {
    ngx_log_error(NGX_LOG_ERR, log, 0, "ngx_metrics: %s", err);
    return;
  }
  fprintf(stderr, "num symbols: %d\n", num_symbols);
}

ngx_metric_t *get_metric_for_symbol(ngx_metrics_symbol_t *sym, span_metric_type_e metric_type) {
  if(sym->metrics[metric_type]) {
    /* fprintf(stderr, "symbol %s has metric\n", sym->name); */
    return sym->metrics[metric_type];
  }
  if(num_dynamic_metrics == MAX_DYNAMIC_METRICS) {
    return NULL;
  }
  ngx_metric_t *metric = &dynamic_metrics[num_dynamic_metrics++];

  // construct tags: "{handler="<symbol name"}"
  char tags[256];
  snprintf(tags, sizeof(tags), "{handler=\"%s\"}", sym->name);

  metric->name = METRIC_TYPE_NAMES[metric_type];
  metric->tags = strdup(tags);

  metric->count = 0;
  metric->sum = 0;
  metric->min = 0;
  metric->max = 0;

  sym->metrics[metric_type] = metric;
  return metric;
}

static ngx_span_t *current_span = NULL;

void ngx_metrics_span_enter(ngx_span_t *span, void *handler) {
  span->parent = current_span;
  span->handler = handler;
  span->start_wall_time = ngx_precise_time();
  span->children_wall_time = 0;
  span->start_cpu_time = ngx_cpu_time();
  span->children_cpu_time = 0;
  current_span = span;
}

void ngx_metrics_span_exit(ngx_span_t *span) {
  int64_t total_wall_time = ngx_precise_time() - span->start_wall_time;
  ngx_metrics_report_event_handler_time(span->handler, SPAN_METRIC_TOTAL_WALL_TIME, total_wall_time);
  ngx_metrics_report_event_handler_time(span->handler, SPAN_METRIC_SELF_WALL_TIME, total_wall_time - span->children_wall_time);

  int64_t total_cpu_time = ngx_cpu_time() - span->start_cpu_time;
  ngx_metrics_report_event_handler_time(span->handler, SPAN_METRIC_TOTAL_CPU_TIME, total_cpu_time);
  ngx_metrics_report_event_handler_time(span->handler, SPAN_METRIC_SELF_CPU_TIME, total_cpu_time - span->children_cpu_time);

  if(span->parent) {
    span->parent->children_wall_time += total_wall_time;
    span->parent->children_cpu_time += total_cpu_time;
  }
  current_span = span->parent;
}
