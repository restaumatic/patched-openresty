#include <ngx_metrics.h>
#include <time.h>
#include <stdint.h>

#define METRIC_INIT(name) { name, 0, 0, 0, 0 }

ngx_metric_t ngx_metric_event_handler_time_ns = METRIC_INIT("event_handler_time_ns");
ngx_metric_t ngx_metric_open_and_stat_file_time_ns = METRIC_INIT("open_and_stat_file_time_ns");
ngx_metric_t ngx_metric_event_loop_latency_ns = METRIC_INIT("event_loop_latency_ns");

ngx_metric_t *metrics[] = {
  &ngx_metric_event_handler_time_ns,
  &ngx_metric_open_and_stat_file_time_ns,
  &ngx_metric_event_loop_latency_ns,
};

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
  return sizeof(metrics) / sizeof(metrics[0]);
}

ngx_metric_t *ngx_get_metric(int index) {
  return metrics[index];
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
