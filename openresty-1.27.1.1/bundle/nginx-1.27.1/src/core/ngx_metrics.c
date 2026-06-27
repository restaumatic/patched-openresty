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

static char *read_file_contents(ngx_log_t *log, const char *filename);

#define NUM_SPAN_METRIC_TYPES 2

typedef enum {
    SPAN_METRIC_TOTAL_WALL_TIME = 0,
    SPAN_METRIC_SELF_WALL_TIME
} span_metric_type_e;

#define MAX_DYNAMIC_METRICS (128 * NUM_SPAN_METRIC_TYPES)

static const char *METRIC_TYPE_NAMES[] = {
  "event_handler_total_wall_time_ns",
  "event_handler_self_wall_time_ns",
};

typedef struct {
  const char *name;
  /** Address relative to the base address of the executable */
  uintptr_t address;
  ngx_metric_t *metrics[NUM_SPAN_METRIC_TYPES];
} symbol;

symbol *find_symbol(uintptr_t addr);

ngx_metric_t *get_metric_for_symbol(symbol *sym, span_metric_type_e metric_type);

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
  symbol *sym = find_symbol(relative_addr);
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

// Symbol table for handler name resolution

symbol *symbols = NULL;
int num_symbols = 0;

#define MAX_EXE_FILENAME 256
#define SUFFIX ".symbols"

int count_lines(const char *contents);

bool ngx_metrics_init_symbols(ngx_log_t *log) {
  char filename[MAX_EXE_FILENAME + sizeof(SUFFIX)];
  ssize_t len = readlink("/proc/self/exe", filename, MAX_EXE_FILENAME);
  if(len == -1 || len == MAX_EXE_FILENAME) {
    ngx_log_error(NGX_LOG_ERR, log, 0, "ngx_metrics: Failed to read /proc/self/exe");
    return false;
  }
  strcpy(filename + strlen(filename), SUFFIX);

  char *contents = read_file_contents(log, filename);

  int new_num_symbols = count_lines(contents);

  symbol *new_symbols = malloc(new_num_symbols * sizeof(symbol));

  fprintf(stderr, "num symbols: %d\n", new_num_symbols);

  const char *p = contents;

  // Parse lines in the format:
  // 0000000000049393 t ngx_cleanup_environment_variable
  for(int i = 0; i < new_num_symbols; i++) {
    uintptr_t address;
    char type;
    char name[256];
    int matched = sscanf(p, "%lx %c %s\n", &address, &type, name);
    if(matched != 3) {
      ngx_log_error(NGX_LOG_ERR, log, 0, "ngx_metrics: Failed to parse symbols file");
      free(contents);
      free(new_symbols);
      return false;
    }
    new_symbols[i].name = strdup(name);
    new_symbols[i].address = (uintptr_t) address;
    memset(new_symbols[i].metrics, 0, sizeof(new_symbols[i].metrics));
    p = strchr(p, '\n') + 1;
  }

  free(contents);

  symbols = new_symbols;
  num_symbols = new_num_symbols;

  return true;
}

void ngx_metrics_init(ngx_log_t *log) {
  if(!ngx_metrics_init_symbols(log)) {
    // TODO: exit or what?
    return;
  }
}

// Read the contents of a file into a malloc'd buffer. Returns NULL on error.
// It is the caller's responsibility to free the buffer.
char *read_file_contents(ngx_log_t *log, const char *filename) {
  FILE *f = fopen(filename, "r");

  // get file size
  if(fseek(f, 0, SEEK_END)) {
    ngx_log_error(NGX_LOG_ERR, log, 0, "ngx_metrics: Failed to seek to end of symbols file");
    fclose(f);
    return NULL;
  }
  size_t size = ftell(f);
  rewind(f);

  // allocate a buffer and read the whole contents
  char *buffer = malloc(size);
  if(!buffer) {
    ngx_log_error(NGX_LOG_ERR, log, 0, "ngx_metrics: Failed to allocate buffer for symbols");
    fclose(f);
    return NULL;
  }
  size_t read = fread(buffer, size, 1, f);
  if(read != 1) {
    ngx_log_error(NGX_LOG_ERR, log, 0,
        "ngx_metrics: Failed to read symbols file (%d bytes read out of %d)",
        read, size);
    fclose(f);
    free(buffer);
    return NULL;
  }

  fclose(f);
  return buffer;
}

int count_lines(const char *contents) {
  int count = 0;
  for(int i = 0; contents[i]; i++) {
    if(contents[i] == '\n') {
      count++;
    }
  }
  return count;
}

int compare_symbols(const void *a, const void *b) {
  symbol *sa = (symbol *) a;
  symbol *sb = (symbol *) b;
  if(sa->address < sb->address) {
    return -1;
  } else if(sa->address > sb->address) {
    return 1;
  } else {
    return 0;
  }
}

symbol *find_symbol(uintptr_t addr) {
  if(!symbols) {
    return NULL;
  }
  // Work around the weird API of bsearch
  symbol fake_symbol = { .name = NULL, .address = addr };
  return bsearch(&fake_symbol, symbols, num_symbols, sizeof(symbol), compare_symbols);
}

ngx_metric_t *get_metric_for_symbol(symbol *sym, span_metric_type_e metric_type) {
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
  span->start_time = ngx_precise_time();
  span->children_time = 0;
  current_span = span;
}

void ngx_metrics_span_exit(ngx_span_t *span) {
  int64_t total_time = ngx_precise_time() - span->start_time;
  int64_t self_time = total_time - span->children_time;
  ngx_metrics_report_event_handler_time(span->handler, SPAN_METRIC_TOTAL_WALL_TIME, total_time);
  ngx_metrics_report_event_handler_time(span->handler, SPAN_METRIC_SELF_WALL_TIME, self_time);

  if(span->parent) {
    span->parent->children_time += total_time;
  }
  current_span = span->parent;
}
